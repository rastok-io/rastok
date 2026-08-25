//! Stable Connector identities, role requirements, and exchange lifecycles.
//!
//! A Connector id is semantic plan data.  It names a contract independently
//! of the crate, Driver binary, Host or Worker that implements it. Roles and
//! exchange lifecycles are likewise contract facts: runtime registration may
//! satisfy them, but may not reinterpret them.

use alloc::boxed::Box;
use core::fmt;

/// The standard Process contract's stable major-version identity.
pub const PROCESS_CONNECTOR_ID: &str = "rastok/process@1";

/// The standard single-file contract's stable major-version identity.
pub const FILE_CONNECTOR_ID: &str = "rastok/file@1";

/// The standard outbound HTTP call contract's stable identity.
pub const HTTP_CLIENT_CONNECTOR_ID: &str = "rastok/http-client@1";

/// The standard inbound HTTP request contract's stable identity.
pub const HTTP_SERVER_CONNECTOR_ID: &str = "rastok/http-server@1";

/// The standard outbound WebSocket session contract's stable identity.
pub const WEBSOCKET_CLIENT_CONNECTOR_ID: &str = "rastok/websocket-client@1";

/// The standard accepted WebSocket session contract's stable identity.
pub const WEBSOCKET_SERVER_CONNECTOR_ID: &str = "rastok/websocket-server@1";

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
/// more dot-separated lowercase labels; the name is one lowercase label;
/// labels may contain ASCII digits and internal `-`; the positive major has
/// no leading zero.  The constructor never normalizes aliases.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectorId(Box<str>);

impl ConnectorId {
    /// Parses one exact canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns the first structural category that makes `spelling`
    /// non-canonical.
    pub fn parse(spelling: &str) -> Result<Self, ConnectorIdError> {
        let Some((qualified, major)) = spelling.rsplit_once('@') else {
            return Err(ConnectorIdError::Shape);
        };
        if qualified.contains('@') {
            return Err(ConnectorIdError::Shape);
        }
        let Some((namespace, name)) = qualified.split_once('/') else {
            return Err(ConnectorIdError::Shape);
        };
        if name.contains('/') {
            return Err(ConnectorIdError::Shape);
        }
        if namespace.is_empty() || namespace.split('.').any(|label| !canonical_label(label)) {
            return Err(ConnectorIdError::Namespace);
        }
        if !canonical_label(name) {
            return Err(ConnectorIdError::Name);
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
            return Err(ConnectorIdError::Major);
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

impl fmt::Display for ConnectorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ConnectorId {
    type Error = ConnectorIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

/// Why a Connector contract identity is not canonical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorIdError {
    /// The `/` and `@` framing is absent or ambiguous.
    Shape,
    /// The namespace is not a dot-separated sequence of canonical labels.
    Namespace,
    /// The contract name is not one canonical label.
    Name,
    /// The major is not a positive canonical `u32` decimal.
    Major,
}

impl fmt::Display for ConnectorIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Shape => "expected exactly `<namespace>/<name>@<major>`",
            Self::Namespace => "the namespace is not canonical lowercase dotted labels",
            Self::Name => "the name is not one canonical lowercase label",
            Self::Major => "the major is not a positive canonical u32 decimal",
        })
    }
}

impl core::error::Error for ConnectorIdError {}

/// The cardinality and terminal lifecycle of one Connector interaction.
///
/// This is orthogonal to [`ConnectorRoles`]: roles say which side may move a
/// fact across the world boundary, while this value says whether those facts
/// are independent events, one bounded call, or members of one long-lived
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConnectorExchange {
    /// One spontaneous inbound fact with no per-event terminal obligation.
    Event,
    /// One admitted operation with zero or more outputs and exactly one exit.
    Call,
    /// One open scope carrying messages and exactly one terminal exit.
    Session,
}

impl ConnectorExchange {
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

impl fmt::Display for ConnectorExchange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Event => "event",
            Self::Call => "call",
            Self::Session => "session",
        })
    }
}

/// The runtime roles one Connector contract requires.
///
/// There is deliberately no empty value: a Connector without either world
/// role is an Operator or Graph, not a Connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConnectorRoles {
    /// Proposes external arrivals to Rastok.
    Inbound,
    /// Executes already-journaled calls into the external world.
    Outbound,
    /// One implementation state exposes both roles.
    InboundAndOutbound,
}

impl ConnectorRoles {
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

impl fmt::Display for ConnectorRoles {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Inbound => "inbound",
            Self::Outbound => "outbound",
            Self::InboundAndOutbound => "inbound+outbound",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectorExchange, ConnectorId, ConnectorIdError, ConnectorRoles};

    #[test]
    fn connector_ids_accept_one_canonical_spelling_only() {
        for valid in [
            "rastok/process@1",
            "org.example/http-client@12",
            "a/x1@4294967295",
        ] {
            assert_eq!(ConnectorId::parse(valid).unwrap().as_str(), valid);
        }
        for (invalid, reason) in [
            ("Process", ConnectorIdError::Shape),
            ("rastok/process", ConnectorIdError::Shape),
            ("rastok/process@1@2", ConnectorIdError::Shape),
            ("rastok/more/process@1", ConnectorIdError::Shape),
            ("Rastok/process@1", ConnectorIdError::Namespace),
            ("rastok./process@1", ConnectorIdError::Namespace),
            ("rastok/Process@1", ConnectorIdError::Name),
            ("rastok/-process@1", ConnectorIdError::Name),
            ("rastok/process@0", ConnectorIdError::Major),
            ("rastok/process@01", ConnectorIdError::Major),
            ("rastok/process@4294967296", ConnectorIdError::Major),
        ] {
            assert_eq!(ConnectorId::parse(invalid), Err(reason), "{invalid}");
        }
    }

    #[test]
    fn role_inclusion_has_no_empty_or_partial_duplex_state() {
        assert!(ConnectorRoles::Inbound.includes(ConnectorRoles::Inbound));
        assert!(!ConnectorRoles::Inbound.includes(ConnectorRoles::Outbound));
        assert!(!ConnectorRoles::Inbound.includes(ConnectorRoles::InboundAndOutbound));
        assert!(ConnectorRoles::Outbound.includes(ConnectorRoles::Outbound));
        assert!(ConnectorRoles::InboundAndOutbound.includes(ConnectorRoles::Inbound));
        assert!(ConnectorRoles::InboundAndOutbound.includes(ConnectorRoles::Outbound));
        assert!(ConnectorRoles::InboundAndOutbound.includes(ConnectorRoles::InboundAndOutbound));
        for role in [
            ConnectorRoles::Inbound,
            ConnectorRoles::Outbound,
            ConnectorRoles::InboundAndOutbound,
        ] {
            assert_eq!(ConnectorRoles::from_tag(role.tag()), Some(role));
        }
        assert_eq!(ConnectorRoles::from_tag(0), None);
        assert_eq!(ConnectorRoles::from_tag(4), None);
    }

    #[test]
    fn exchange_tags_are_closed_and_extend_only() {
        for exchange in [
            ConnectorExchange::Event,
            ConnectorExchange::Call,
            ConnectorExchange::Session,
        ] {
            assert_eq!(ConnectorExchange::from_tag(exchange.tag()), Some(exchange));
        }
        assert_eq!(ConnectorExchange::from_tag(0), None);
        assert_eq!(ConnectorExchange::from_tag(4), None);
    }
}
