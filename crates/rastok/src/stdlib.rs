//! The standard catalogue — concrete instances of the
//! [`crate::contract`] metamodel.
//!
//! Governing rule: [`stdlib.md`] — the ten-case Connector template and
//! the standard contracts. Contracts are data: the checker validates
//! declarations against these sheets without linking any Driver, and
//! Drivers implement them later. The current catalogue ships the
//! `Process`, `File`, HTTP and WebSocket sheets; the remaining names are
//! declared so they refuse explicitly rather than accidentally.
//!
//! [`stdlib.md`]: https://github.com/rastok/rastok/blob/main/docs/spec/stdlib.md

use crate::connector::{
    ConnectorExchange, ConnectorRoles, FILE_CONNECTOR_ID, HTTP_CLIENT_CONNECTOR_ID,
    HTTP_SERVER_CONNECTOR_ID, PROCESS_CONNECTOR_ID, WEBSOCKET_CLIENT_CONNECTOR_ID,
    WEBSOCKET_SERVER_CONNECTOR_ID,
};
use crate::contract::{
    AttributeSpec, ConnectorContract, ContractCatalog, DefaultValue, EvaluationSite,
    FrontendContract, FrontendFraming, FrontendReader, PortContract, PortDataType, PortTypeChoice,
    PortTypeRule, Presence, Profile, ProfileSelector, ProfileSupport, ValueDomain,
};
use crate::node::Direction;
use crate::plan::UnwiredPolicy;

/// The `Process` sheet of `stdlib.md` §3.3 on the D-2026-08-12-1
/// metamodel: the common attributes, and the `mode` selector picking
/// `job` (default, available) or `service` (known to the contract,
/// deferred by this build — its attribute NAMES are normative, their
/// domains deferred-only).
pub const PROCESS: ConnectorContract = ConnectorContract {
    id: PROCESS_CONNECTOR_ID,
    name: "Process",
    roles: ConnectorRoles::Outbound,
    exchange: ConnectorExchange::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: &[
                    PortTypeChoice {
                        word: "lines",
                        ty: PortDataType::Text,
                    },
                    PortTypeChoice {
                        word: "jsonl",
                        ty: PortDataType::Variant,
                    },
                    PortTypeChoice {
                        word: "bytes",
                        ty: PortDataType::Bytes,
                    },
                ],
            },
        },
        PortContract {
            name: "out",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: &[
                    PortTypeChoice {
                        word: "lines",
                        ty: PortDataType::Text,
                    },
                    PortTypeChoice {
                        word: "jsonl",
                        ty: PortDataType::Variant,
                    },
                    PortTypeChoice {
                        word: "bytes",
                        ty: PortDataType::Bytes,
                    },
                ],
            },
        },
        PortContract {
            name: "err",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            ty: PortTypeRule::AttributeSelected {
                attribute: "err_codec",
                choices: &[
                    PortTypeChoice {
                        word: "lines",
                        ty: PortDataType::Text,
                    },
                    PortTypeChoice {
                        word: "bytes",
                        ty: PortDataType::Bytes,
                    },
                ],
            },
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            // One exact union is shared by the contract, v7 port table,
            // completion gate and Process Driver.
            ty: PortTypeRule::Fixed(PortDataType::ProcessExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "args",
            presence: Presence::Defaulted(DefaultValue::Expression("[]")),
            domain: ValueDomain::TextList,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["deterministic", "probabilistic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            // The framing is the sheet's most semantic decision —
            // bytes, digest, reply type and error surface all follow
            // it — so it is never chosen silently (D-2026-08-11-1,
            // reaffirmed by D-2026-08-12-1).
            name: "codec",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["bytes", "jsonl", "lines"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            // stderr is an independent physical channel. It never inherits
            // the stdout codec and defaults to one text value per line.
            name: "err_codec",
            presence: Presence::Defaulted(DefaultValue::Choice("lines")),
            domain: ValueDomain::Choice(&["bytes", "lines"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "command",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "cwd",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "effect",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["idempotent", "compensable", "irreversible"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            // A declared budget; absence means NO budget — a real
            // Optional, not a hidden default.
            name: "timeout",
            presence: Presence::Optional,
            domain: ValueDomain::Duration,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "env",
            presence: Presence::Optional,
            domain: ValueDomain::TextMap,
            latest_site: EvaluationSite::Instance,
        },
    ],
    selector: Some(ProfileSelector {
        attribute: "mode",
        default: Some("job"),
        profiles: &[
            Profile {
                word: "job",
                support: ProfileSupport::Available,
                attributes: &[
                    AttributeSpec {
                        // Judged by `contract::byte_size` — the same
                        // parser the Driver enforces the budget
                        // with (D-2026-08-12-1, review round).
                        name: "max_output",
                        presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
                        domain: ValueDomain::ByteSize,
                        latest_site: EvaluationSite::Plan,
                    },
                    AttributeSpec {
                        // Bypassing a detached one-shot is
                        // meaningless: the domain has ONE word, and
                        // the diagnostic says "fixed to".
                        name: "on_timeout",
                        presence: Presence::Defaulted(DefaultValue::Choice("kill")),
                        domain: ValueDomain::Choice(&["kill"]),
                        latest_site: EvaluationSite::Plan,
                    },
                ],
            },
            Profile {
                word: "service",
                support: ProfileSupport::Deferred("the Process service mode"),
                attributes: &[
                    AttributeSpec {
                        // `correlate = data.id` — an expression over
                        // runtime data, not a boot-phase constant.
                        name: "correlate",
                        presence: Presence::Optional,
                        domain: ValueDomain::ExprPath,
                        latest_site: EvaluationSite::Invocation,
                    },
                    AttributeSpec {
                        // The restart policy struct (`{ backoff = …,
                        // max_attempts = … }`) — typed with the
                        // service slice.
                        name: "restart",
                        presence: Presence::Optional,
                        domain: ValueDomain::Struct,
                        latest_site: EvaluationSite::Instance,
                    },
                ],
            },
        ],
    }),
};

/// The one-file outbound contract. One invocation either reads one complete
/// bounded byte value or atomically replaces one file; directory operations
/// and observation streams are separate contracts, never hidden modes.
pub const FILE: ConnectorContract = ConnectorContract {
    id: FILE_CONNECTOR_ID,
    name: "File",
    roles: ConnectorRoles::Outbound,
    exchange: ConnectorExchange::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "mode",
                choices: &[
                    PortTypeChoice {
                        word: "read",
                        ty: PortDataType::Variant,
                    },
                    PortTypeChoice {
                        word: "replace",
                        ty: PortDataType::Bytes,
                    },
                    // The deferred selector word keeps this table total. It
                    // promises no future trigger shape: lifting an inbound
                    // watch requires an explicit contract-version decision.
                    PortTypeChoice {
                        word: "watch",
                        ty: PortDataType::Variant,
                    },
                ],
            },
        },
        PortContract {
            name: "out",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            ty: PortTypeRule::Fixed(PortDataType::Bytes),
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            ty: PortTypeRule::Fixed(PortDataType::FileExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "path",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Invocation,
        },
        AttributeSpec {
            // Read and atomic replace are idempotent for the same sealed
            // request. Append and directory mutation are not this contract.
            name: "effect",
            presence: Presence::Defaulted(DefaultValue::Choice("idempotent")),
            domain: ValueDomain::Choice(&["idempotent"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "behavior",
            presence: Presence::Defaulted(DefaultValue::Choice("deterministic")),
            domain: ValueDomain::Choice(&["deterministic"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "timeout",
            presence: Presence::Optional,
            domain: ValueDomain::Duration,
            latest_site: EvaluationSite::Instance,
        },
    ],
    selector: Some(ProfileSelector {
        attribute: "mode",
        // Reading versus replacing is user intent and never inferred from the
        // payload or wiring.
        default: None,
        profiles: &[
            Profile {
                word: "read",
                support: ProfileSupport::Available,
                attributes: &[AttributeSpec {
                    name: "max_read",
                    presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
                    domain: ValueDomain::ByteSize,
                    latest_site: EvaluationSite::Plan,
                }],
            },
            Profile {
                word: "replace",
                support: ProfileSupport::Available,
                attributes: &[],
            },
            Profile {
                word: "watch",
                support: ProfileSupport::Deferred("the File watch profile"),
                attributes: &[],
            },
        ],
    }),
};

const NETWORK_BODY_TYPES: &[PortTypeChoice] = &[
    PortTypeChoice {
        word: "bytes",
        ty: PortDataType::Bytes,
    },
    PortTypeChoice {
        word: "json",
        ty: PortDataType::Variant,
    },
    PortTypeChoice {
        word: "text",
        ty: PortDataType::Text,
    },
];

const NETWORK_VERSIONS: &[&str] = &["http/1.1", "http/2", "http/3"];
const ALL_NETWORK_VERSIONS: &str = r#"["http/3", "http/2", "http/1.1"]"#;

/// One outbound HTTP request and its portable terminal fact. The negotiated
/// wire version is physical observation; it never selects another contract.
pub const HTTP_CLIENT: ConnectorContract = ConnectorContract {
    id: HTTP_CLIENT_CONNECTOR_ID,
    name: "HttpClient",
    roles: ConnectorRoles::Outbound,
    exchange: ConnectorExchange::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: NETWORK_BODY_TYPES,
            },
        },
        PortContract {
            name: "out",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::SameAs("in"),
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
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
            name: "effect",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["idempotent", "compensable", "irreversible"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "headers",
            presence: Presence::Optional,
            domain: ValueDomain::HeaderMap,
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

/// One inbound HTTP request. `ack` is executable without a response-routing
/// seam; `respond` is part of the same stable contract but remains named and
/// deferred until H5 can hold and settle the remote request obligation.
pub const HTTP_SERVER: ConnectorContract = ConnectorContract {
    id: HTTP_SERVER_CONNECTOR_ID,
    name: "HttpServer",
    roles: ConnectorRoles::Inbound,
    exchange: ConnectorExchange::Call,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: NETWORK_BODY_TYPES,
            },
        },
        PortContract {
            name: "out",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::SameAs("in"),
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            ty: PortTypeRule::Fixed(PortDataType::HttpServerExit),
        },
    ],
    common: &[
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
            name: "effect",
            presence: Presence::Defaulted(DefaultValue::Choice("none")),
            domain: ValueDomain::Choice(&["none"]),
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "headers",
            presence: Presence::Optional,
            domain: ValueDomain::HeaderMap,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "listen",
            presence: Presence::Required,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "max_body",
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
    selector: Some(ProfileSelector {
        attribute: "mode",
        default: Some("ack"),
        profiles: &[
            Profile {
                word: "ack",
                support: ProfileSupport::Available,
                attributes: &[],
            },
            Profile {
                word: "respond",
                support: ProfileSupport::Deferred("the HttpServer respond mode (M1.6 H5)"),
                attributes: &[
                    AttributeSpec {
                        name: "correlate",
                        presence: Presence::Optional,
                        domain: ValueDomain::ExprPath,
                        latest_site: EvaluationSite::Invocation,
                    },
                    AttributeSpec {
                        name: "timeout",
                        presence: Presence::Required,
                        domain: ValueDomain::Duration,
                        latest_site: EvaluationSite::Instance,
                    },
                ],
            },
        ],
    }),
};

/// One client-opened WebSocket session: many complete messages in both
/// directions and exactly one terminal session exit.
pub const WEBSOCKET_CLIENT: ConnectorContract = ConnectorContract {
    id: WEBSOCKET_CLIENT_CONNECTOR_ID,
    name: "WebSocketClient",
    roles: ConnectorRoles::InboundAndOutbound,
    exchange: ConnectorExchange::Session,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: NETWORK_BODY_TYPES,
            },
        },
        PortContract {
            name: "out",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::SameAs("in"),
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
            ty: PortTypeRule::Fixed(PortDataType::WebSocketExit),
        },
    ],
    common: &[
        AttributeSpec {
            name: "auth_bearer",
            presence: Presence::Optional,
            domain: ValueDomain::Text,
            latest_site: EvaluationSite::Instance,
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
            name: "effect",
            presence: Presence::Required,
            domain: ValueDomain::Choice(&["idempotent", "compensable", "irreversible"]),
            latest_site: EvaluationSite::Plan,
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
            domain: ValueDomain::HeaderMap,
            latest_site: EvaluationSite::Instance,
        },
        AttributeSpec {
            name: "max_message",
            presence: Presence::Defaulted(DefaultValue::Expression(r#""8MiB""#)),
            domain: ValueDomain::ByteSize,
            latest_site: EvaluationSite::Plan,
        },
        AttributeSpec {
            name: "subprotocols",
            presence: Presence::Optional,
            domain: ValueDomain::TextList,
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

/// One accepted WebSocket session. The listener and every live session share
/// one both-role Driver state; physical connection identity never enters the
/// graph contract.
pub const WEBSOCKET_SERVER: ConnectorContract = ConnectorContract {
    id: WEBSOCKET_SERVER_CONNECTOR_ID,
    name: "WebSocketServer",
    roles: ConnectorRoles::InboundAndOutbound,
    exchange: ConnectorExchange::Session,
    ports: &[
        PortContract {
            name: "in",
            direction: Direction::Input,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::AttributeSelected {
                attribute: "codec",
                choices: NETWORK_BODY_TYPES,
            },
        },
        PortContract {
            name: "out",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RequireWire,
            ty: PortTypeRule::SameAs("in"),
        },
        PortContract {
            name: "exit",
            direction: Direction::Output,
            unwired: UnwiredPolicy::RootSink,
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
            name: "effect",
            presence: Presence::Defaulted(DefaultValue::Choice("none")),
            domain: ValueDomain::Choice(&["none"]),
            latest_site: EvaluationSite::Plan,
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
            domain: ValueDomain::HeaderMap,
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

/// The catalogue names declared by `stdlib.md` whose Drivers are not
/// in the M0 build.
const DECLARED: &[&str] = &["Clock", "Calendar", "Memory"];

const M0_FRONTENDS: &[FrontendContract] = &[
    FrontendContract {
        name: "json",
        reader: FrontendReader::Json,
    },
    FrontendContract {
        name: "text",
        reader: FrontendReader::Text,
    },
    FrontendContract {
        name: "base64",
        reader: FrontendReader::Base64,
    },
    FrontendContract {
        name: "jq",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the jq guest frontend",
        },
    },
    FrontendContract {
        name: "sql",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the SQL guest frontend",
        },
    },
    FrontendContract {
        name: "bash",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the bash guest frontend",
        },
    },
    FrontendContract {
        name: "python",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Python guest frontend",
        },
    },
    FrontendContract {
        name: "rust",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Rust guest frontend",
        },
    },
    FrontendContract {
        name: "avro",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Avro data frontend",
        },
    },
    FrontendContract {
        name: "protobuf",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Protobuf data frontend",
        },
    },
    FrontendContract {
        name: "parquet",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the Parquet data frontend",
        },
    },
    FrontendContract {
        name: "msgpack",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the MessagePack data frontend",
        },
    },
    FrontendContract {
        name: "csv",
        reader: FrontendReader::Deferred {
            framing: FrontendFraming::QuotedBraced,
            construct: "the CSV data frontend",
        },
    },
];

impl ContractCatalog {
    /// The M0 registry through its fallible door — the same
    /// validation any extension walks.
    ///
    /// # Errors
    ///
    /// The first [`crate::contract::SheetDefect`] of the standard
    /// catalogue.
    pub fn try_m0() -> Result<Self, crate::contract::SheetDefect> {
        let base = Self {
            connectors: std::collections::BTreeMap::new(),
            declared: DECLARED.to_vec(),
            frontends: Vec::new(),
        };
        // The standard catalogue walks through the SAME door as any
        // extension — never an unvalidated authority.
        M0_FRONTENDS
            .iter()
            .cloned()
            .try_fold(base, Self::try_with_frontend)?
            .try_with(PROCESS)?
            .try_with(FILE)?
            .try_with(HTTP_CLIENT)?
            .try_with(HTTP_SERVER)?
            .try_with(WEBSOCKET_CLIENT)?
            .try_with(WEBSOCKET_SERVER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::Availability;

    #[test]
    fn the_catalogue_resolves_the_three_availabilities() {
        let catalogue = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert!(matches!(
            catalogue.connector("Process"),
            Availability::Available(contract) if contract.name == "Process"
        ));
        assert!(matches!(
            catalogue.connector("HttpServer"),
            Availability::Available(contract) if contract.id == HTTP_SERVER_CONNECTOR_ID
        ));
        assert_eq!(catalogue.connector("Clock"), Availability::Declared);
        assert_eq!(catalogue.connector("Postgres"), Availability::Unknown);
    }

    #[test]
    fn the_process_sheet_holds_its_own_door() {
        // The sheet validates at the registry door — defaults
        // compiled, profiles coherent.
        assert!(crate::contract::validate_sheet(&PROCESS).is_ok());
        let process = PROCESS;
        let job = process.profile("job").unwrap();
        let effect = process.attribute(Some(job), "effect").unwrap();
        assert_eq!(effect.presence, Presence::Required);
        assert!(process.attribute(Some(job), "max_output").is_some());
        assert!(process.attribute(Some(job), "restart").is_none());
        assert_eq!(
            process.owning_profile("restart").map(|p| p.word),
            Some("service")
        );
        let ports = PROCESS
            .resolve_ports(&std::collections::BTreeMap::from([
                (String::from("codec"), String::from("lines")),
                (String::from("err_codec"), String::from("lines")),
            ]))
            .expect("the effective Process sheet resolves");
        assert_eq!(ports.len(), 4);
        let process_exit = crate::boundary::process_exit_type().expect("the target type is valid");
        assert!(
            ports
                .iter()
                .any(|port| port.port.name() == "exit" && port.ty.as_ref() == Some(&process_exit))
        );
    }

    #[test]
    fn the_file_sheet_holds_its_own_door() {
        assert!(crate::contract::validate_sheet(&FILE).is_ok());
        let read = FILE.profile("read").expect("the read profile");
        assert!(FILE.attribute(Some(read), "max_read").is_some());
        assert_eq!(
            FILE.attribute(Some(read), "path")
                .expect("the common path")
                .latest_site,
            EvaluationSite::Invocation
        );
        let ports = FILE
            .resolve_ports(&std::collections::BTreeMap::from([(
                String::from("mode"),
                String::from("read"),
            )]))
            .expect("the effective File sheet resolves");
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn every_network_sheet_row_agrees_with_the_core_mirror() {
        let sheets = [
            &HTTP_CLIENT,
            &HTTP_SERVER,
            &WEBSOCKET_CLIENT,
            &WEBSOCKET_SERVER,
        ];
        for sheet in sheets {
            crate::contract::validate_sheet(sheet).expect("the network sheet holds its door");
            let id = crate::connector::ConnectorId::parse(sheet.id).expect("a canonical id");
            for attribute in sheet.common {
                assert_eq!(
                    crate::boundary::network_attribute_site(&id, attribute.name),
                    Some(attribute.latest_site),
                    "{}.{}",
                    sheet.name,
                    attribute.name
                );
            }
            if let Some(selector) = &sheet.selector {
                assert_eq!(
                    crate::boundary::network_attribute_site(&id, selector.attribute),
                    Some(EvaluationSite::Plan)
                );
                for profile in selector.profiles {
                    for attribute in profile.attributes {
                        assert_eq!(
                            crate::boundary::network_attribute_site(&id, attribute.name),
                            Some(attribute.latest_site),
                            "{}({}).{}",
                            sheet.name,
                            profile.word,
                            attribute.name
                        );
                    }
                }
            }

            for codec in ["bytes", "json", "text"] {
                let mut words = std::collections::BTreeMap::from([(
                    String::from("codec"),
                    String::from(codec),
                )]);
                if sheet.id == HTTP_SERVER_CONNECTOR_ID {
                    words.insert(String::from("mode"), String::from("ack"));
                }
                let ports = sheet
                    .resolve_ports(&words)
                    .expect("the effective network ports resolve");
                for port in ports {
                    assert_eq!(
                        port.ty,
                        crate::boundary::network_port_type(&id, port.port.name(), Some(codec)),
                        "{}.{} under {codec}",
                        sheet.name,
                        port.port
                    );
                }
            }
        }
    }

    #[test]
    fn the_network_metadata_and_terminal_unions_are_closed() {
        assert_eq!(
            crate::plan::standard_ext_field_type("http_status"),
            Some(crate::datatype::DataType::INT)
        );
        assert_eq!(
            crate::plan::standard_ext_field_type("http_headers"),
            Some(crate::plan::http_headers_type())
        );
        let canonical_headers = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("set-cookie"),
            crate::value::Value::Array(vec![
                crate::value::Value::Bytes(b"a=1".to_vec()),
                crate::value::Value::Bytes(b"b=2".to_vec()),
            ]),
        )]));
        assert!(crate::plan::http_headers_value_holds(&canonical_headers));
        let uppercase = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("Content-Type"),
            crate::value::Value::Array(vec![crate::value::Value::Bytes(
                b"application/json".to_vec(),
            )]),
        )]));
        assert!(!crate::plan::http_headers_value_holds(&uppercase));
        let empty_values = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from("accept"),
            crate::value::Value::Array(Vec::new()),
        )]));
        assert!(!crate::plan::http_headers_value_holds(&empty_values));
        let pseudo_field = crate::value::Value::Map(std::collections::BTreeMap::from([(
            String::from(":status"),
            crate::value::Value::Array(vec![crate::value::Value::Bytes(b"200".to_vec())]),
        )]));
        assert!(!crate::plan::http_headers_value_holds(&pseudo_field));
        let client = crate::boundary::http_client_exit_type().expect("the client exit union");
        let server = crate::boundary::http_server_exit_type().expect("the server exit union");
        let websocket = crate::boundary::websocket_exit_type().expect("the session exit union");
        let case_names = |ty: crate::datatype::DataType| match ty {
            crate::datatype::DataType::TaggedUnion(union) => union
                .cases()
                .iter()
                .map(|case| case.name().to_string())
                .collect::<Vec<_>>(),
            other => panic!("expected a tagged union, got {other:?}"),
        };
        assert_eq!(
            case_names(client),
            [
                "BodyTooLarge",
                "Completed",
                "ConnectFailed",
                "DecodeFailed",
                "DnsFailed",
                "OutcomeUnknown",
                "ProtocolFailed",
                "TimeoutExpired",
                "TlsFailed",
            ]
        );
        assert_eq!(
            case_names(server),
            [
                "AuthFailed",
                "Completed",
                "MalformedRequest",
                "OutcomeUnknown",
                "Overloaded",
                "PeerDisconnected",
                "RequestExpired",
            ]
        );
        assert_eq!(
            case_names(websocket),
            [
                "Closed",
                "ConnectFailed",
                "DecodeFailed",
                "DnsFailed",
                "HandshakeRejected",
                "MessageTooLarge",
                "OutcomeUnknown",
                "ProtocolFailed",
                "TimeoutExpired",
                "TlsFailed",
                "TransportLost",
            ]
        );
    }

    #[test]
    fn frontends_share_one_contract_registry() {
        let registry = ContractCatalog::try_m0().expect("the standard catalogue holds");
        assert_eq!(
            registry.frontend("json").map(|frontend| frontend.reader),
            Some(FrontendReader::Json)
        );
        assert!(matches!(
            registry.frontend("jq").map(|frontend| frontend.reader),
            Some(FrontendReader::Deferred { .. })
        ));
        assert_eq!(
            registry
                .frontend("text")
                .map(|frontend| frontend.reader.result_type()),
            Some(Some(crate::datatype::DataType::Text))
        );
        assert_eq!(registry.frontend("starlark"), None);
    }
}
