//! Portable http client Contract rules.

use super::HTTP_CLIENT_CONNECTOR_ID;
use super::*;
use super::{ALL_NETWORK_VERSIONS, NETWORK_BODY_TYPES, NETWORK_VERSIONS};
use crate::connector::contract::{
    AttributeSpec, ConnectorContract, DefaultValue, EvaluationSite, PortContract, PortDataType,
    PortTypeRule, Presence, ValueDomain,
};
use crate::connector::{ConnectorDirection, ExchangeKind};
use crate::datatype::{DataType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField};
use crate::node::PortDirection;

/// Portable terminal union of one outbound HTTP call.
pub fn http_client_exit_type() -> Result<DataType, TypeError> {
    let completed = TaggedCase::new(
        "Completed",
        StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?,
    )?;
    let body_too_large = TaggedCase::new(
        "BodyTooLarge",
        StructType::new(alloc::vec![TypeField::new("limit", DataType::UINT)])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        completed,
        TaggedCase::new(
            "ServerError",
            StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?
        )?,
        empty_case("DnsFailed")?,
        empty_case("ConnectFailed")?,
        empty_case("TlsFailed")?,
        empty_case("ProtocolFailed")?,
        empty_case("TimeoutExpired")?,
        body_too_large,
        empty_case("DecodeFailed")?,
        empty_case("OutcomeUnknown")?,
    ])?))
}

/// Portable terminal facts of one outbound HTTP call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpClientExit {
    /// A usable response outside 500–599. Non-2xx is not a transport failure.
    Completed {
        /// Three-digit HTTP status code.
        status: u16,
    },
    /// A usable decoded HTTP 500–599 response, after external visibility.
    ServerError {
        /// The observed HTTP status, retained unchanged.
        status: u16,
    },
    /// Name resolution failed before request bytes became visible.
    DnsFailed,
    /// The transport connection could not be established.
    ConnectFailed,
    /// Verified TLS negotiation failed.
    TlsFailed,
    /// HTTP framing or negotiation failed.
    ProtocolFailed,
    /// The scheduler's call budget won the close race.
    TimeoutExpired,
    /// The declared request- or response-body bound was exceeded.
    BodyTooLarge {
        /// Declared body limit.
        limit: u64,
    },
    /// A complete body did not inhabit the selected codec.
    DecodeFailed,
    /// Cancellation settled without a portable terminal observation.
    OutcomeUnknown,
}

impl HttpClientExit {
    /// Classifies an already admitted and decoded response. Decode failure
    /// must be selected before this function, even for a server-error status.
    #[must_use]
    pub const fn response(status: u16) -> Self {
        if status >= 500 && status <= 599 {
            Self::ServerError { status }
        } else {
            Self::Completed { status }
        }
    }

    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::Value;
        match self {
            Self::Completed { status } => tagged_value(
                "Completed",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::ServerError { status } => tagged_value(
                "ServerError",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::DnsFailed => tagged_value("DnsFailed", Vec::new()),
            Self::ConnectFailed => tagged_value("ConnectFailed", Vec::new()),
            Self::TlsFailed => tagged_value("TlsFailed", Vec::new()),
            Self::ProtocolFailed => tagged_value("ProtocolFailed", Vec::new()),
            Self::TimeoutExpired => tagged_value("TimeoutExpired", Vec::new()),
            Self::BodyTooLarge { limit } => tagged_value(
                "BodyTooLarge",
                alloc::vec![(String::from("limit"), Value::Uint(limit))],
            ),
            Self::DecodeFailed => tagged_value("DecodeFailed", Vec::new()),
            Self::OutcomeUnknown => tagged_value("OutcomeUnknown", Vec::new()),
        }
    }
}

/// Standard declarative Contract; physical Drivers implement these resolved rules.
pub const HTTP_CLIENT: ConnectorContract = ConnectorContract {
    id: HTTP_CLIENT_CONNECTOR_ID,
    name: "HttpClient",
    roles: ConnectorDirection::Outbound,
    exchange: ExchangeKind::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: PortDirection::Input,

            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: NETWORK_BODY_TYPES,
            },
        },
        PortContract {
            name: "out",
            direction: PortDirection::Output,

            ty: PortTypeRule::JsonOutput {
                attribute: "codec",
                choices: NETWORK_BODY_TYPES,
            },
        },
        PortContract {
            name: "exit",
            direction: PortDirection::Output,

            ty: PortTypeRule::Fixed(PortDataType::HttpClientExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "auth_bearer",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["deterministic", "probabilistic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "codec",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["bytes", "json", "text"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "json_profile",
            presence: Presence::Optional,
            domain: ValueDomain::Choice(&["projected"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "idempotent",
            presence: Presence::Defaulted(DefaultValue::Expression("false")),
            domain: ValueDomain::Boolean,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "headers",
            presence: Presence::Optional,
            domain: ValueDomain::TypedHttpHeaders,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "custom_headers",
            presence: Presence::Optional,
            domain: ValueDomain::CustomHttpHeaders,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "idempotency_key",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "max_body",
            presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
            domain: ValueDomain::ByteSize,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "method",
            presence: Presence::Defaulted(DefaultValue::Expression(r#""GET""#)),
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "timeout",
            presence: Presence::Required,
            domain: ValueDomain::Duration,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "tls_ca",
            presence: Presence::Optional,
            domain: ValueDomain::Bytes,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "tls_certificate",
            presence: Presence::Optional,
            domain: ValueDomain::Bytes,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "tls_private_key",
            presence: Presence::Optional,
            domain: ValueDomain::Bytes,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "url",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "versions",
            presence: Presence::Defaulted(DefaultValue::Expression(ALL_NETWORK_VERSIONS)),
            domain: ValueDomain::ChoiceList(NETWORK_VERSIONS),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "zero_rtt",
            presence: Presence::Defaulted(DefaultValue::Choice("disabled")),
            domain: ValueDomain::Choice(&["disabled"]),
            latest_site: EvaluationSite::Plan,
        },
    ],
    selector: None,
};
