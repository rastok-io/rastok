//! Effectless Lab bindings for the two WebSocket session contracts.
//!
//! They exercise the generic H1 Session seam and the exact H2 codecs without
//! opening a socket. H3/H6 replace only the external-world observation; the
//! contract ids, shared both-role object and message framing stay put.

use crate::boundary::{
    InboundConnectorDriver, InboundDecision, InboundProposal, InboundProposalRef,
    OutboundConnectorDriver, PreparedRequest, RequestPreparation, SessionConnectorDriver,
    SessionDriverEvent, SessionOpenIntent, SessionRef, SessionWriteIntent,
};
use crate::connector::{ConnectorId, ConnectorRoles};
use crate::envelope::{Data, Envelope};
use crate::plan::{AttrSetting, NodeKind, ValidatedExecutionPlan};
use crate::value::Value;
use std::collections::BTreeMap;

struct WebSocketLabFactory {
    id: ConnectorId,
}

struct WebSocketLabDriver {
    specs: BTreeMap<crate::node::Path, crate::plan::ConnectorSpec>,
}

impl WebSocketLabDriver {
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

    fn message_bytes(
        &self,
        connector: &crate::node::Path,
        data: &Data,
    ) -> Result<(String, Vec<u8>), crate::boundary::WebSocketExit> {
        let Some(spec) = self.specs.get(connector) else {
            return Err(crate::boundary::WebSocketExit::ProtocolFailed);
        };
        let Some(AttrSetting::Word(codec)) = spec.attribute("codec") else {
            return Err(crate::boundary::WebSocketExit::ProtocolFailed);
        };
        crate::boundary::network_payload_bytes(codec, data)
            .map(|bytes| (codec.clone(), bytes))
            .map_err(|_| crate::boundary::WebSocketExit::DecodeFailed)
    }

    fn prepare_message(
        &self,
        connector: &crate::node::Path,
        data: &Data,
    ) -> Result<PreparedRequest, crate::boundary::WebSocketExit> {
        let (codec, body) = self.message_bytes(connector, data)?;
        let descriptor = Value::Struct(vec![
            (String::from("body"), Value::Bytes(body)),
            (String::from("codec"), Value::Text(codec)),
        ]);
        Ok(PreparedRequest::seal(crate::canonical_cbor::encode(
            &crate::identity::encoding::value_tagged(&descriptor),
        )))
    }

    fn prepare_open(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        resolved: &BTreeMap<String, Value>,
    ) -> Result<PreparedRequest, crate::boundary::WebSocketExit> {
        let Some(spec) = self.specs.get(connector) else {
            return Err(crate::boundary::WebSocketExit::ProtocolFailed);
        };
        if spec.contract.as_str() != crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID {
            return Err(crate::boundary::WebSocketExit::ProtocolFailed);
        }
        let (codec, initial_message) = self.message_bytes(connector, data)?;
        let headers =
            Self::value(spec, resolved, "headers").unwrap_or_else(|| Value::Map(BTreeMap::new()));
        if !crate::plan::http_headers_value_holds(&headers) {
            return Err(crate::boundary::WebSocketExit::ProtocolFailed);
        }
        let Some(Value::Text(url)) = Self::value(spec, resolved, "url") else {
            return Err(crate::boundary::WebSocketExit::ProtocolFailed);
        };
        let descriptor = Value::Struct(vec![
            (
                String::from("auth_bearer"),
                Self::value(spec, resolved, "auth_bearer").unwrap_or(Value::Null),
            ),
            (String::from("codec"), Value::Text(codec)),
            (String::from("headers"), headers),
            (
                String::from("initial_message"),
                Value::Bytes(initial_message),
            ),
            (
                String::from("subprotocols"),
                Self::value(spec, resolved, "subprotocols")
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            ),
            (
                String::from("tls_ca"),
                Self::value(spec, resolved, "tls_ca").unwrap_or(Value::Null),
            ),
            (
                String::from("tls_certificate"),
                Self::value(spec, resolved, "tls_certificate").unwrap_or(Value::Null),
            ),
            (
                String::from("tls_private_key"),
                Self::value(spec, resolved, "tls_private_key").unwrap_or(Value::Null),
            ),
            (String::from("url"), Value::Text(url)),
            (
                String::from("versions"),
                Self::value(spec, resolved, "versions")
                    .ok_or(crate::boundary::WebSocketExit::ProtocolFailed)?,
            ),
            (
                String::from("zero_rtt"),
                Self::value(spec, resolved, "zero_rtt")
                    .ok_or(crate::boundary::WebSocketExit::ProtocolFailed)?,
            ),
        ]);
        Ok(PreparedRequest::seal(crate::canonical_cbor::encode(
            &crate::identity::encoding::value_tagged(&descriptor),
        )))
    }

    fn preparation(
        result: Result<PreparedRequest, crate::boundary::WebSocketExit>,
    ) -> RequestPreparation {
        match result {
            Ok(prepared) => RequestPreparation::Prepared(prepared),
            Err(exit) => {
                RequestPreparation::Rejected(crate::boundary::Completion::websocket_exit(exit))
            }
        }
    }
}

impl InboundConnectorDriver for WebSocketLabDriver {
    fn proposal(&mut self) -> Option<InboundProposal> {
        None
    }

    fn decide(&mut self, _: InboundProposalRef, _: InboundDecision) {}
}

impl OutboundConnectorDriver for WebSocketLabDriver {
    fn call(&mut self) -> Option<&mut dyn crate::boundary::CallConnectorDriver> {
        None
    }

    fn session(&mut self) -> Option<&mut dyn SessionConnectorDriver> {
        Some(self)
    }
}

impl SessionConnectorDriver for WebSocketLabDriver {
    fn prepare_session_open(
        &mut self,
        connector: &crate::node::Path,
        data: &Data,
        settings: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        Self::preparation(self.prepare_open(connector, data, settings))
    }

    fn open_session(&mut self, _: &SessionOpenIntent, _: &Envelope) {}

    fn prepare_session_write(
        &mut self,
        connector: &crate::node::Path,
        _: SessionRef,
        data: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        Self::preparation(self.prepare_message(connector, data))
    }

    fn write_session(&mut self, _: &SessionWriteIntent, _: &Envelope) {}

    fn session_events(&mut self) -> Vec<SessionDriverEvent> {
        Vec::new()
    }
}

impl super::BoundConnectorDriver for WebSocketLabDriver {
    fn inbound(&mut self) -> Option<&mut dyn InboundConnectorDriver> {
        Some(self)
    }

    fn outbound(&mut self) -> Option<&mut dyn OutboundConnectorDriver> {
        Some(self)
    }
}

impl super::ConnectorDriverFactory for WebSocketLabFactory {
    fn connector_id(&self) -> &ConnectorId {
        &self.id
    }

    fn roles(&self) -> ConnectorRoles {
        ConnectorRoles::InboundAndOutbound
    }

    fn bind<'plan>(
        &self,
        plan: &'plan ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
    ) -> Result<Box<dyn super::BoundConnectorDriver + 'plan>, super::DriverFactoryError> {
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
                        "the WebSocket binding path `{path}` is not a connector"
                    ))
                })?;
            if &spec.contract != self.connector_id() {
                return Err(super::DriverFactoryError::new(format!(
                    "the WebSocket binder for `{}` received `{}` at `{path}`",
                    self.id, spec.contract
                )));
            }
            if !matches!(spec.attribute("codec"), Some(AttrSetting::Word(_))) {
                return Err(super::DriverFactoryError::new(format!(
                    "the WebSocket codec of `{path}` is unresolved"
                )));
            }
            specs.insert(path.clone(), spec.clone());
        }
        Ok(Box::new(WebSocketLabDriver { specs }))
    }
}

pub(crate) fn lab_factories()
-> Result<Vec<Box<dyn super::ConnectorDriverFactory>>, super::DriverFactoryError> {
    [
        crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID,
        crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID,
    ]
    .into_iter()
    .map(|spelling| {
        ConnectorId::parse(spelling)
            .map(|id| {
                Box::new(WebSocketLabFactory { id }) as Box<dyn super::ConnectorDriverFactory>
            })
            .map_err(|error| super::DriverFactoryError::new(error.to_string()))
    })
    .collect()
}
