//! Standard websocket server Contract.

use super::WEBSOCKET_SERVER_CONNECTOR_ID;
use super::{ALL_NETWORK_VERSIONS, NETWORK_BODY_TYPES, NETWORK_VERSIONS};
use crate::connector::contract::{
    AttributeSpec, ConnectorContract, DefaultValue, EvaluationSite, PortContract, PortDataType,
    PortTypeRule, Presence, ValueDomain,
};
use crate::connector::{ConnectorDirection, ExchangeKind};
use crate::node::PortDirection;

/// Standard declarative Contract; physical Drivers implement these resolved rules.
pub const WEBSOCKET_SERVER: ConnectorContract = ConnectorContract {
    id: WEBSOCKET_SERVER_CONNECTOR_ID,
    name: "WebSocketServer",
    roles: ConnectorDirection::InboundAndOutbound,
    exchange: ExchangeKind::Session,
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

            ty: PortTypeRule::SameAs("in"),
        },
        PortContract {
            name: "exit",
            direction: PortDirection::Output,

            ty: PortTypeRule::Fixed(PortDataType::WebSocketExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "allowed_origins",
            presence: Presence::Optional,
            domain: ValueDomain::TextList,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "auth_bearer",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Defaulted(DefaultValue::Choice("deterministic")),
            domain: ValueDomain::Choice(&["deterministic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "codec",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["bytes", "json", "text"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "correlate",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            name: "handshake_timeout",
            presence: Presence::Required,
            domain: ValueDomain::Duration,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "headers",
            presence: Presence::Optional,
            domain: ValueDomain::TypedHttpHeaders,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "custom_headers",
            presence: Presence::Optional,
            domain: ValueDomain::CustomHttpHeaders,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "listen",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "max_message",
            presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
            domain: ValueDomain::ByteSize,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "path",
            presence: Presence::Defaulted(DefaultValue::Expression(r#""/""#)),
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "subprotocols",
            presence: Presence::Optional,
            domain: ValueDomain::TextList,
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
