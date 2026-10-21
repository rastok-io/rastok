//! Shared validation for the standard HTTP and WebSocket Contracts.

use super::*;
use crate::connector::contract::{ModeSupport, Presence, ValueDomain};
use crate::plan::EvaluationSite;

fn setting_holds(domain: &ValueDomain, setting: &crate::plan::AttrSetting) -> bool {
    match (domain, setting) {
        (ValueDomain::Choice(words), crate::plan::AttrSetting::Word(word)) => {
            words.contains(&word.as_str())
        }
        (ValueDomain::ExprPath, crate::plan::AttrSetting::Value(crate::value::Value::Text(_))) => {
            true
        }
        (
            ValueDomain::Duration,
            crate::plan::AttrSetting::Value(crate::value::Value::Duration(value)),
        ) => *value > 0,
        (_, crate::plan::AttrSetting::Value(value)) => domain.admits(value),
        (_, crate::plan::AttrSetting::Expr { expr, .. }) => domain.admits_type(&expr.ty),
        _ => false,
    }
}

pub(super) fn validate(
    node: &Node,
    spec: &crate::plan::ConnectorSpec,
    loop_spec: Option<&crate::plan::LoopSpec>,
) -> Result<bool, PlanValidationError> {
    let contract = spec.contract.as_str();
    let Some(shape) = super::contract(&spec.contract).filter(|definition| {
        matches!(
            definition.id,
            HTTP_CLIENT_CONNECTOR_ID
                | HTTP_SERVER_CONNECTOR_ID
                | WEBSOCKET_CLIENT_CONNECTOR_ID
                | WEBSOCKET_SERVER_CONNECTOR_ID
        )
    }) else {
        return Ok(false);
    };
    let mode = spec.attribute("mode").and_then(|setting| match setting {
        crate::plan::AttrSetting::Word(word) => shape.profile(word),
        _ => None,
    });
    if spec.roles != shape.roles {
        return Err(PlanValidationError::ContractRoleMismatch {
            node: node.path.clone(),
        });
    }
    if spec.exchange != shape.exchange {
        return Err(PlanValidationError::ContractExchangeMismatch {
            node: node.path.clone(),
        });
    }
    let axes_hold = match contract {
        crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID
        | crate::connector::stdlib::WEBSOCKET_SERVER_CONNECTOR_ID => {
            spec.behavior == node::Behavior::Deterministic
        }
        _ => true,
    };
    if !axes_hold {
        return Err(PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: String::from("behavior"),
        });
    }

    for attribute in shape
        .common
        .iter()
        .chain(mode.into_iter().flat_map(|mode| mode.attributes))
    {
        if attribute.name != "behavior"
            && attribute.name != "idempotent"
            && !matches!(attribute.presence, Presence::Optional)
            && spec.attribute(attribute.name).is_none()
        {
            return Err(PlanValidationError::MissingContractAttribute {
                node: node.path.clone(),
                attribute: attribute.name,
            });
        }
    }
    for (name, setting) in &spec.attributes {
        let holds = if shape
            .selector
            .as_ref()
            .is_some_and(|selector| selector.attribute == name)
        {
            matches!(setting, crate::plan::AttrSetting::Word(word) if shape.profile(word).is_some_and(|mode| matches!(mode.support, ModeSupport::Available)))
        } else {
            name != "behavior"
                && shape
                    .attribute(mode, name)
                    .is_some_and(|attribute| setting_holds(&attribute.domain, setting))
        };
        if !holds {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: name.clone(),
            });
        }
    }
    if contract == crate::connector::stdlib::HTTP_SERVER_CONNECTOR_ID {
        match spec.attribute("mode") {
            Some(crate::plan::AttrSetting::Word(mode)) if mode == "ack" => {
                if spec.attribute("correlate").is_some() || spec.attribute("timeout").is_some() {
                    return Err(PlanValidationError::ContractAttributeMismatch {
                        node: node.path.clone(),
                        attribute: String::from("mode/correlate/timeout"),
                    });
                }
            }
            Some(crate::plan::AttrSetting::Word(mode)) if mode == "respond" => {
                if spec.attribute("timeout").is_none() {
                    return Err(PlanValidationError::MissingContractAttribute {
                        node: node.path.clone(),
                        attribute: "timeout",
                    });
                }
                if let Some(crate::plan::AttrSetting::Expr { site, expr }) =
                    spec.attribute("correlate")
                    && (*site != EvaluationSite::Invocation || expr.ty != DataType::Text)
                {
                    return Err(PlanValidationError::ContractAttributeMismatch {
                        node: node.path.clone(),
                        attribute: String::from("correlate"),
                    });
                }
            }
            Some(_) => {
                return Err(PlanValidationError::ContractAttributeMismatch {
                    node: node.path.clone(),
                    attribute: String::from("mode"),
                });
            }
            None => {
                return Err(PlanValidationError::MissingContractAttribute {
                    node: node.path.clone(),
                    attribute: "mode",
                });
            }
        }
    }
    if spec.attribute("tls_certificate").is_some() != spec.attribute("tls_private_key").is_some() {
        return Err(PlanValidationError::ContractAttributeMismatch {
            node: node.path.clone(),
            attribute: String::from("tls_certificate/tls_private_key"),
        });
    }

    let codec = match spec.attribute("codec") {
        Some(crate::plan::AttrSetting::Word(codec)) => Some(codec.as_str()),
        _ => None,
    };
    for declared in shape.ports {
        let (name, direction) = (declared.name, declared.direction);
        let Some(port) = node.ports.iter().find(|port| port.port.name() == name) else {
            return Err(PlanValidationError::MissingContractPort {
                node: node.path.clone(),
                port: name,
            });
        };
        let json_output = contract == crate::connector::stdlib::HTTP_CLIENT_CONNECTOR_ID
            && name == "out"
            && codec == Some("json");
        if name == "out"
            && spec.attribute("json_profile").is_some()
            && (!json_output || !matches!(port.ty, Some(DataType::Struct(_))))
        {
            return Err(PlanValidationError::ContractAttributeMismatch {
                node: node.path.clone(),
                attribute: String::from("json_profile"),
            });
        }
        let type_holds = if json_output {
            port.ty.as_ref().is_some_and(crate::json::supports_type)
        } else {
            port.ty
                == effective_contract_port(
                    node,
                    loop_spec,
                    name,
                    crate::connector::stdlib::network_port_type(&spec.contract, name, codec),
                )
        };
        if port.direction != direction || !type_holds {
            return Err(PlanValidationError::ContractPortMismatch {
                node: node.path.clone(),
                port: port.port.clone(),
            });
        }
    }
    if let Some(port) = node.ports.iter().find(|port| {
        !shape
            .ports
            .iter()
            .any(|declared| declared.name == port.port.name())
    }) {
        return Err(PlanValidationError::UnexpectedContractPort {
            node: node.path.clone(),
            port: port.port.clone(),
        });
    }
    Ok(true)
}
