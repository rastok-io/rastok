//! HttpServer contract binding and effectless Lab Driver.
//!
//! Scenario injections enter through the ordinary Lab boundary. The hosted
//! listener uses the same plan rows without sharing a source module with
//! HttpClient.

#[cfg(feature = "hosted-http12")]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::boundary::{
    InboundCallConnectorDriver, InboundCallEvent, InboundCallResponse, InboundCallResponseStart,
    InboundConnectorDriver, InboundDecision, InboundProposal, InboundProposalRef,
};
use crate::connector::{ConnectorId, ConnectorRoles};
use crate::plan::ValidatedExecutionPlan;
#[cfg(feature = "hosted-http12")]
use crate::plan::{AttrSetting, NodeKind};
#[cfg(feature = "hosted-http12")]
use crate::value::Value;

#[cfg(feature = "hosted-http12")]
pub use crate::connectors::network::http_server::{HttpServerConfig, HttpServerFactory};

#[cfg(feature = "hosted-http12")]
use super::http_shared::HttpVersion;

/// One statically bindable route of the hosted server family.
#[cfg(feature = "hosted-http12")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpServerRouteBinding {
    pub(crate) auth_bearer: Option<String>,
    pub(crate) codec: String,
    pub(crate) listen: String,
    pub(crate) max_body: u64,
    pub(crate) node: crate::node::Path,
    pub(crate) path: String,
    pub(crate) response_headers: BTreeMap<String, Vec<Vec<u8>>>,
    pub(crate) tls_certificate: Option<Vec<u8>>,
    pub(crate) tls_private_key: Option<Vec<u8>>,
    pub(crate) versions: Vec<HttpVersion>,
}

#[cfg(feature = "hosted-http12")]
fn static_value(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<Value>, super::DriverFactoryError> {
    match spec.attribute(name) {
        Some(AttrSetting::Value(value)) => Ok(Some(value.clone())),
        Some(AttrSetting::Expr { .. }) => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer setting `{name}` at `{}` must resolve before listener binding",
            spec.contract
        ))),
        Some(AttrSetting::Word(_)) => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer setting `{name}` has the wrong value form"
        ))),
        None => Ok(None),
    }
}

#[cfg(feature = "hosted-http12")]
fn required_text(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<String, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Text(value)) => Ok(value),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer requires a resolved text `{name}` setting"
        ))),
    }
}

#[cfg(feature = "hosted-http12")]
fn optional_text(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<String>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Text(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer `{name}` setting is not text"
        ))),
    }
}

#[cfg(feature = "hosted-http12")]
fn optional_bytes(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<Vec<u8>>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Bytes(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted HttpServer `{name}` setting is not bytes"
        ))),
    }
}

#[cfg(feature = "hosted-http12")]
fn response_headers(
    spec: &crate::plan::ConnectorSpec,
) -> Result<BTreeMap<String, Vec<Vec<u8>>>, super::DriverFactoryError> {
    let typed = static_value(spec, "headers")?.unwrap_or_else(|| Value::Struct(Vec::new()));
    let custom =
        static_value(spec, "custom_headers")?.unwrap_or_else(|| Value::Map(BTreeMap::new()));
    let Some(Value::Map(headers)) = crate::plan::canonical_request_headers(&typed, &custom) else {
        return Err(super::DriverFactoryError::new(
            "the hosted HttpServer response headers are not canonical",
        ));
    };
    headers
        .into_iter()
        .map(|(name, values)| {
            let Value::Array(values) = values else {
                return Err(super::DriverFactoryError::new(
                    "the hosted HttpServer response-header row is not an array",
                ));
            };
            let values = values
                .into_iter()
                .map(|value| match value {
                    Value::Bytes(value) => Ok(value),
                    _ => Err(super::DriverFactoryError::new(
                        "the hosted HttpServer response-header value is not bytes",
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((name, values))
        })
        .collect()
}

/// Resolves every static listener and route row before any socket opens.
#[cfg(feature = "hosted-http12")]
pub(crate) fn bind_routes(
    plan: &ValidatedExecutionPlan,
    nodes: &[crate::node::Path],
) -> Result<Vec<HttpServerRouteBinding>, super::DriverFactoryError> {
    let expected: BTreeSet<_> = nodes.iter().cloned().collect();
    let mut routes = Vec::new();
    for path in nodes {
        let spec = plan
            .plan()
            .nodes
            .iter()
            .find(|node| &node.path == path)
            .and_then(|node| match &node.kind {
                NodeKind::Connector(spec) => Some(spec),
                NodeKind::Operator => None,
            })
            .ok_or_else(|| {
                super::DriverFactoryError::new(format!(
                    "the HttpServer binding path `{path}` is not a connector"
                ))
            })?;
        if spec.contract.as_str() != crate::connector::HTTP_SERVER_CONNECTOR_ID {
            return Err(super::DriverFactoryError::new(format!(
                "the HttpServer binder received `{}` at `{path}`",
                spec.contract
            )));
        }
        if !matches!(spec.attribute("mode"), Some(AttrSetting::Word(mode)) if mode == "ack") {
            return Err(super::DriverFactoryError::new(format!(
                "the hosted HttpServer at `{path}` requires the executable `ack` mode"
            )));
        }
        let codec = match spec.attribute("codec") {
            Some(AttrSetting::Word(codec))
                if matches!(codec.as_str(), "bytes" | "json" | "text") =>
            {
                codec.clone()
            }
            _ => {
                return Err(super::DriverFactoryError::new(format!(
                    "the hosted HttpServer codec at `{path}` is unresolved"
                )));
            }
        };
        let listen = required_text(spec, "listen")?;
        let route_path = required_text(spec, "path")?;
        if !route_path.starts_with('/') || route_path.contains(['?', '#']) {
            return Err(super::DriverFactoryError::new(format!(
                "the hosted HttpServer path `{route_path}` at `{path}` is not an absolute path"
            )));
        }
        let max_body = match static_value(spec, "max_body")? {
            Some(Value::Text(value)) => crate::connector::byte_size(&value),
            _ => None,
        }
        .ok_or_else(|| {
            super::DriverFactoryError::new(format!(
                "the hosted HttpServer body bound at `{path}` is unresolved"
            ))
        })?;
        let versions = match static_value(spec, "versions")? {
            Some(Value::Array(values)) => values
                .into_iter()
                .map(|value| match value {
                    Value::Text(value) | Value::Enum(value) => HttpVersion::parse(&value),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>(),
            _ => None,
        }
        .filter(|versions| !versions.is_empty())
        .ok_or_else(|| {
            super::DriverFactoryError::new(format!(
                "the hosted HttpServer version preference at `{path}` is unresolved"
            ))
        })?;
        routes.push(HttpServerRouteBinding {
            auth_bearer: optional_text(spec, "auth_bearer")?,
            codec,
            listen,
            max_body,
            node: path.clone(),
            path: route_path,
            response_headers: response_headers(spec)?,
            tls_certificate: optional_bytes(spec, "tls_certificate")?,
            tls_private_key: optional_bytes(spec, "tls_private_key")?,
            versions,
        });
    }
    let actual: BTreeSet<_> = routes.iter().map(|route| route.node.clone()).collect();
    if expected != actual {
        return Err(super::DriverFactoryError::new(
            "the HttpServer binder received an incomplete connector family",
        ));
    }
    Ok(routes)
}

#[derive(Default)]
struct PassiveInbound;

impl InboundConnectorDriver for PassiveInbound {
    fn proposal(&mut self) -> Option<InboundProposal> {
        None
    }

    fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {}

    fn call(&mut self) -> Option<&mut dyn InboundCallConnectorDriver> {
        Some(self)
    }
}

impl InboundCallConnectorDriver for PassiveInbound {
    fn start_response(&mut self, _: &InboundCallResponse) -> InboundCallResponseStart {
        InboundCallResponseStart::Pending
    }

    fn events(&mut self) -> Vec<InboundCallEvent> {
        Vec::new()
    }
}

struct HttpServerLabFactory {
    id: ConnectorId,
}

impl super::ConnectorDriverFactory for HttpServerLabFactory {
    fn connector_id(&self) -> &ConnectorId {
        &self.id
    }

    fn roles(&self) -> ConnectorRoles {
        ConnectorRoles::Inbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<Box<dyn super::BoundConnectorDriver + 'plan>, super::DriverFactoryError> {
        let expected: BTreeSet<_> = nodes.iter().collect();
        let actual: BTreeSet<_> = plan
            .plan()
            .nodes
            .iter()
            .filter(|node| {
                node.connector().is_some_and(|spec| {
                    spec.contract.as_str() == crate::connector::HTTP_SERVER_CONNECTOR_ID
                })
            })
            .map(|node| &node.path)
            .collect();
        if expected != actual {
            return Err(super::DriverFactoryError::new(
                "the HttpServer Lab binder received an incomplete family",
            ));
        }
        Ok(Box::new(PassiveInbound))
    }
}

impl super::BoundConnectorDriver for PassiveInbound {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }

    fn outbound(&mut self) -> Option<&mut dyn crate::boundary::OutboundConnectorDriver> {
        None
    }
}

pub(crate) fn lab_factory()
-> Result<Box<dyn super::ConnectorDriverFactory>, super::DriverFactoryError> {
    let id = ConnectorId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID)
        .map_err(|error| super::DriverFactoryError::new(error.to_string()))?;
    Ok(Box::new(HttpServerLabFactory { id }))
}
