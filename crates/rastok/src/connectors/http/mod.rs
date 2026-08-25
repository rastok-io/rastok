//! Pure HTTP contract bindings used by the Lab before the hosted socket
//! engine lands in H3.
//!
//! The outbound request descriptor is already family-owned and deterministic:
//! mocks and the future real Driver must agree on the exact method, URL,
//! headers and body covered by the request digest. The server twin is a
//! deliberately passive inbound Driver; scenario injections enter through
//! the ordinary Lab boundary, never through a fake socket.

use crate::boundary::{
    Completion, HttpClientExit, InboundConnectorDriver, InboundDecision, InboundProposal,
    InboundProposalRef, PreparedRequest, RequestPreparation, SchedulerCompletion,
};
use crate::connector::{ConnectorId, ConnectorRoles};
use crate::envelope::Data;
use crate::plan::{AttrSetting, NodeKind, ValidatedExecutionPlan};
use crate::value::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
struct HttpClientRequestBindings {
    specs: BTreeMap<crate::node::Path, crate::plan::ConnectorSpec>,
}

impl HttpClientRequestBindings {
    fn try_bind(
        plan: &ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<Self, super::DriverFactoryError> {
        let mut specs = BTreeMap::new();
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
                        "the HttpClient binding path `{path}` is not a connector"
                    ))
                })?;
            if spec.contract.as_str() != crate::connector::HTTP_CLIENT_CONNECTOR_ID {
                return Err(super::DriverFactoryError::new(format!(
                    "the HttpClient binder received `{}` at `{path}`",
                    spec.contract
                )));
            }
            specs.insert(path.clone(), spec.clone());
        }
        Ok(Self { specs })
    }

    fn value(
        spec: &crate::plan::ConnectorSpec,
        resolved: &BTreeMap<String, Value>,
        name: &str,
    ) -> Option<Value> {
        match spec.attribute(name) {
            Some(AttrSetting::Word(word)) => Some(Value::Enum(word.clone())),
            Some(AttrSetting::Value(value)) => Some(value.clone()),
            Some(AttrSetting::Expr { .. }) => resolved.get(name).cloned(),
            None => None,
        }
    }

    fn text(
        spec: &crate::plan::ConnectorSpec,
        resolved: &BTreeMap<String, Value>,
        name: &str,
    ) -> Result<String, String> {
        match Self::value(spec, resolved, name) {
            Some(Value::Text(value)) => Ok(value),
            _ => Err(format!("the resolved HTTP `{name}` setting is not text")),
        }
    }

    fn prepare(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        resolved: &BTreeMap<String, Value>,
    ) -> Result<PreparedRequest, String> {
        let spec = self
            .specs
            .get(connector)
            .ok_or_else(|| format!("no HttpClient binding owns `{connector}`"))?;
        let codec = match spec.attribute("codec") {
            Some(AttrSetting::Word(codec)) => codec.as_str(),
            _ => return Err(String::from("the HttpClient codec is unresolved")),
        };
        let body = crate::boundary::network_payload_bytes(codec, data)?;
        let headers =
            Self::value(spec, resolved, "headers").unwrap_or_else(|| Value::Map(BTreeMap::new()));
        if !crate::plan::http_headers_value_holds(&headers) {
            return Err(String::from(
                "HTTP headers require lowercase field names and ordered byte values",
            ));
        }
        let request = Value::Struct(vec![
            (
                String::from("auth_bearer"),
                Self::value(spec, resolved, "auth_bearer").unwrap_or(Value::Null),
            ),
            (String::from("body"), Value::Bytes(body)),
            (String::from("codec"), Value::Text(codec.to_string())),
            (String::from("headers"), headers),
            (
                String::from("idempotency_key"),
                Self::value(spec, resolved, "idempotency_key").unwrap_or(Value::Null),
            ),
            (
                String::from("method"),
                Value::Text(Self::text(spec, resolved, "method")?),
            ),
            (
                String::from("url"),
                Value::Text(Self::text(spec, resolved, "url")?),
            ),
        ]);
        let bytes =
            crate::canonical_cbor::encode(&crate::identity::encoding::value_tagged(&request));
        Ok(PreparedRequest::seal(bytes))
    }
}

impl super::ScriptedConnectorProtocol for HttpClientRequestBindings {
    fn prepare_request(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        settings: &BTreeMap<String, Value>,
    ) -> RequestPreparation {
        match self.prepare(connector, data, settings) {
            Ok(prepared) => RequestPreparation::Prepared(prepared),
            Err(_) => RequestPreparation::Rejected(Completion::http_client_exit(
                HttpClientExit::DecodeFailed,
            )),
        }
    }

    fn scheduler_completion(&self, completion: SchedulerCompletion) -> Completion {
        Completion::http_client_exit(match completion {
            SchedulerCompletion::TimeoutExpired => HttpClientExit::TimeoutExpired,
            SchedulerCompletion::OutcomeUnknown => HttpClientExit::OutcomeUnknown,
        })
    }
}

pub(super) fn bind_scripted_protocol(
    plan: &ValidatedExecutionPlan,
    nodes: &[crate::node::Path],
) -> Result<Box<dyn super::ScriptedConnectorProtocol>, super::DriverFactoryError> {
    HttpClientRequestBindings::try_bind(plan, nodes)
        .map(|bindings| Box::new(bindings) as Box<dyn super::ScriptedConnectorProtocol>)
}

#[derive(Default)]
struct PassiveInbound;

impl InboundConnectorDriver for PassiveInbound {
    fn proposal(&mut self) -> Option<InboundProposal> {
        None
    }

    fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {}
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

pub(crate) fn lab_factories()
-> Result<Vec<Box<dyn super::ConnectorDriverFactory>>, super::DriverFactoryError> {
    let id = ConnectorId::parse(crate::connector::HTTP_SERVER_CONNECTOR_ID)
        .map_err(|error| super::DriverFactoryError::new(error.to_string()))?;
    Ok(vec![Box::new(HttpServerLabFactory { id })])
}
