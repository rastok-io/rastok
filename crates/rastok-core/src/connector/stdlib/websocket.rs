//! Portable websocket Contract rules.

use super::*;
use crate::datatype::{DataType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField};

/// Portable terminal union shared by WebSocket client and server sessions.
pub fn websocket_exit_type() -> Result<DataType, TypeError> {
    let closed = TaggedCase::new(
        "Closed",
        StructType::new(alloc::vec![
            TypeField::new("code", protocol_code_type()),
            TypeField::new("reason", DataType::Text),
        ])?,
    )?;
    let handshake_rejected = TaggedCase::new(
        "HandshakeRejected",
        StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?,
    )?;
    let message_too_large = TaggedCase::new(
        "MessageTooLarge",
        StructType::new(alloc::vec![TypeField::new("limit", DataType::UINT)])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        closed,
        empty_case("DnsFailed")?,
        empty_case("ConnectFailed")?,
        empty_case("TlsFailed")?,
        handshake_rejected,
        empty_case("ProtocolFailed")?,
        empty_case("TransportLost")?,
        empty_case("TimeoutExpired")?,
        message_too_large,
        empty_case("DecodeFailed")?,
        empty_case("OutcomeUnknown")?,
    ])?))
}

/// Portable terminal facts shared by WebSocket client and server sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketExit {
    /// The close handshake supplied a portable code and UTF-8 reason.
    Closed {
        /// RFC 6455 close code.
        code: u16,
        /// Peer-provided close reason, possibly empty.
        reason: String,
    },
    /// Name resolution failed while opening a client session.
    DnsFailed,
    /// The transport connection could not be established.
    ConnectFailed,
    /// Verified TLS negotiation failed.
    TlsFailed,
    /// The HTTP handshake refused the session.
    HandshakeRejected {
        /// HTTP refusal status.
        status: u16,
    },
    /// WebSocket framing or negotiation failed.
    ProtocolFailed,
    /// An established transport disappeared without a clean close.
    TransportLost,
    /// The session/opening budget won its close race.
    TimeoutExpired,
    /// One complete message exceeded the declared bound.
    MessageTooLarge {
        /// Declared complete-message limit.
        limit: u64,
    },
    /// A complete message did not inhabit the selected codec.
    DecodeFailed,
    /// Recovery cannot prove which terminal observation the peer saw.
    OutcomeUnknown,
}

impl WebSocketExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::Value;
        match self {
            Self::Closed { code, reason } => tagged_value(
                "Closed",
                alloc::vec![
                    (String::from("code"), Value::Int(i64::from(code))),
                    (String::from("reason"), Value::Text(reason)),
                ],
            ),
            Self::DnsFailed => tagged_value("DnsFailed", Vec::new()),
            Self::ConnectFailed => tagged_value("ConnectFailed", Vec::new()),
            Self::TlsFailed => tagged_value("TlsFailed", Vec::new()),
            Self::HandshakeRejected { status } => tagged_value(
                "HandshakeRejected",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::ProtocolFailed => tagged_value("ProtocolFailed", Vec::new()),
            Self::TransportLost => tagged_value("TransportLost", Vec::new()),
            Self::TimeoutExpired => tagged_value("TimeoutExpired", Vec::new()),
            Self::MessageTooLarge { limit } => tagged_value(
                "MessageTooLarge",
                alloc::vec![(String::from("limit"), Value::Uint(limit))],
            ),
            Self::DecodeFailed => tagged_value("DecodeFailed", Vec::new()),
            Self::OutcomeUnknown => tagged_value("OutcomeUnknown", Vec::new()),
        }
    }
}
