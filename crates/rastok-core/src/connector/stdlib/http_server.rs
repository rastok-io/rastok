//! Portable http server Contract rules.

use super::HTTP_SERVER_CONNECTOR_ID;
use super::*;
use super::{ALL_NETWORK_VERSIONS, NETWORK_BODY_TYPES, NETWORK_VERSIONS};
use crate::connector::contract::{
    AttributeSpec, ConnectorContract, ContractMode, DefaultValue, EvaluationSite, ModeSelector,
    ModeSupport, PortContract, PortDataType, PortTypeRule, Presence, ValueDomain,
};
use crate::connector::{ConnectorDirection, ExchangeKind};
use crate::datatype::{DataType, StructType, TaggedCase, TaggedUnionType, TypeError, TypeField};
use crate::node::NodeIndex;
use crate::node::PortDirection;
use crate::plan::ExecutionPlan;
use alloc::{collections::BTreeSet, vec};

/// Portable terminal union of one admitted HTTP server request.
pub fn http_server_exit_type() -> Result<DataType, TypeError> {
    let completed = TaggedCase::new(
        "Completed",
        StructType::new(alloc::vec![TypeField::new("status", protocol_code_type())])?,
    )?;
    Ok(DataType::TaggedUnion(TaggedUnionType::new(alloc::vec![
        completed,
        empty_case("RequestExpired")?,
        empty_case("Overloaded")?,
        empty_case("PeerDisconnected")?,
        empty_case("OutcomeUnknown")?,
    ])?))
}

/// Portable terminal facts of one admitted HTTP server request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpServerExit {
    /// The protocol response became terminal.
    Completed {
        /// Status returned to the remote caller.
        status: u16,
    },
    /// A held respond-mode request exceeded its budget.
    RequestExpired,
    /// A committed response could not enter the bounded protocol writer.
    Overloaded,
    /// The peer disappeared before the response became terminal.
    PeerDisconnected,
    /// Recovery cannot prove which terminal observation the peer saw.
    OutcomeUnknown,
}

impl HttpServerExit {
    /// Projects the Driver fact into the ordinary canonical tagged value.
    #[must_use]
    pub fn into_value(self) -> crate::value::Value {
        use crate::value::Value;
        match self {
            Self::Completed { status } => tagged_value(
                "Completed",
                alloc::vec![(String::from("status"), Value::Int(i64::from(status)))],
            ),
            Self::RequestExpired => tagged_value("RequestExpired", Vec::new()),
            Self::Overloaded => tagged_value("Overloaded", Vec::new()),
            Self::PeerDisconnected => tagged_value("PeerDisconnected", Vec::new()),
            Self::OutcomeUnknown => tagged_value("OutcomeUnknown", Vec::new()),
        }
    }
}

/// Standard declarative Contract; physical Drivers implement these resolved rules.
pub const HTTP_SERVER: ConnectorContract = ConnectorContract {
    id: HTTP_SERVER_CONNECTOR_ID,
    name: "HttpServer",
    roles: ConnectorDirection::Inbound,
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

            ty: PortTypeRule::SameAs("in"),
        },
        PortContract {
            name: "exit",
            direction: PortDirection::Output,

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
    selector: Some(ModeSelector {
        attribute: "mode",
        default: Some("ack"),
        profiles: &[
            ContractMode {
                protocol: None,
                word: "ack",
                support: ModeSupport::Available,
                attributes: &[],
            },
            ContractMode {
                protocol: None,
                word: "respond",
                support: ModeSupport::Available,
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

/// Graph responses need respond mode and an explicit correlator across joins.
pub(super) fn validate_response_path(
    plan: &ExecutionPlan,
    slot: NodeIndex,
    node: &Node,
    spec: &ConnectorSpec,
    receives_inbound_call_response: bool,
) -> Result<(), PlanValidationError> {
    if receives_inbound_call_response
        && spec.contract.as_str() == crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID
        && !matches!(spec.attribute("mode"), Some(crate::plan::AttrSetting::Word(mode)) if mode == "respond")
    {
        return Err(PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: String::from("mode"),
        });
    }
    if receives_inbound_call_response
        && spec.contract.as_str() == crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID
        && spec.attribute("correlate").is_none()
        && response_path_crosses_join(plan, slot)
    {
        return Err(PlanValidationError::AmbiguousInboundCallCorrelation {
            node: node.path.clone(),
        });
    }

    Ok(())
}

/// Default inbound-Call lineage is sound only while every upstream handler
/// observes one root. A join deliberately mixes roots and inherits the
/// trigger's origin, so walking the finite DAG backwards is the exact static
/// proof that an explicit correlator is required.
fn response_path_crosses_join(plan: &ExecutionPlan, target: NodeIndex) -> bool {
    let mut pending = vec![target];
    let mut visited = BTreeSet::new();
    while let Some(slot) = pending.pop() {
        if !visited.insert(slot) {
            continue;
        }
        if plan.nodes[slot.index()]
            .handlers()
            .iter()
            .any(|handler| matches!(handler.activation, node::HandlerActivation::Join { .. }))
        {
            return true;
        }
        for source in plan.edges.iter().filter_map(|edge| {
            edge.to
                .as_ref()
                .filter(|endpoint| endpoint.node == slot)
                .map(|_| edge.from.node)
                .filter(|source| *source != NodeIndex::ROOT)
        }) {
            pending.push(source);
        }
    }
    false
}
