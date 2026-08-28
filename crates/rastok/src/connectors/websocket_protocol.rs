//! Effectless Lab bindings for the two WebSocket session contracts.
//!
//! They exercise the generic H1 Session seam and the exact H2 codecs without
//! opening a socket. H3/H6 replace only the external-world observation; the
//! contract ids, shared both-role object and message framing stay put.

use crate::boundary::{
    InboundConnectorDriver, InboundDecision, InboundProposal, InboundProposalRef,
    OutboundConnectorDriver, PreparedRequest, RequestPreparation, SessionConnectorDriver,
    SessionDriverEvent, SessionOpenIntent, SessionRef, SessionStart, SessionWriteIntent,
};
use crate::connector::{ConnectorId, ConnectorRoles};
use crate::envelope::{Data, Envelope};
use crate::plan::{AttrSetting, NodeExecution, ValidatedExecutionPlan};
use crate::value::Value;
use std::collections::BTreeMap;

struct WebSocketLabFactory {
    id: ConnectorId,
}

#[derive(Debug)]
pub(crate) struct WebSocketLabDriver {
    specs: BTreeMap<crate::node::Path, crate::plan::ConnectorSpec>,
}

/// Complete application message sealed before a Session write can become
/// externally visible.
#[cfg(feature = "hosted-websocket")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WebSocketMessage {
    pub(crate) codec: String,
    pub(crate) body: Vec<u8>,
}

/// Exact client handshake material recovered from a journaled opening.
#[cfg(feature = "hosted-websocket")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WebSocketClientOpening {
    pub(crate) auth_bearer: Option<String>,
    pub(crate) codec: String,
    pub(crate) headers: BTreeMap<String, Vec<Vec<u8>>>,
    pub(crate) initial_message: Vec<u8>,
    pub(crate) subprotocols: Vec<String>,
    pub(crate) tls_ca: Option<Vec<u8>>,
    pub(crate) tls_certificate: Option<Vec<u8>>,
    pub(crate) tls_private_key: Option<Vec<u8>>,
    pub(crate) url: String,
    pub(crate) versions: Vec<String>,
}

/// Plan-resolved message and transport limits for one WebSocket node.
#[cfg(feature = "hosted-websocket")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WebSocketPlanBinding {
    pub(crate) codec: String,
    pub(crate) max_message: u64,
    pub(crate) versions: Vec<String>,
}

/// One server route whose listener material is resolved before sockets open.
#[cfg(feature = "hosted-websocket")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WebSocketServerBinding {
    pub(crate) allowed_origins: Vec<String>,
    pub(crate) auth_bearer: Option<String>,
    pub(crate) codec: String,
    pub(crate) headers: BTreeMap<String, Vec<Vec<u8>>>,
    pub(crate) listen: String,
    pub(crate) max_message: u64,
    pub(crate) node: crate::node::Path,
    pub(crate) path: String,
    pub(crate) subprotocols: Vec<String>,
    pub(crate) tls_certificate: Option<Vec<u8>>,
    pub(crate) tls_private_key: Option<Vec<u8>>,
    pub(crate) versions: Vec<String>,
}

#[cfg(feature = "hosted-websocket")]
fn static_value(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<Value>, super::DriverFactoryError> {
    match spec.attribute(name) {
        Some(AttrSetting::Value(value)) => Ok(Some(value.clone())),
        Some(AttrSetting::Word(word)) => Ok(Some(Value::Enum(word.clone()))),
        Some(AttrSetting::Expr { .. }) => Err(super::DriverFactoryError::new(format!(
            "the hosted WebSocketServer setting `{name}` at `{}` must resolve before listener binding",
            spec.contract
        ))),
        None => Ok(None),
    }
}

#[cfg(feature = "hosted-websocket")]
fn required_static_text(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<String, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Text(value)) => Ok(value),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted WebSocketServer requires a resolved text `{name}` setting"
        ))),
    }
}

#[cfg(feature = "hosted-websocket")]
fn optional_static_text(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<String>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Text(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted WebSocketServer `{name}` setting is not text"
        ))),
    }
}

#[cfg(feature = "hosted-websocket")]
fn optional_static_bytes(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Option<Vec<u8>>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(Value::Bytes(value)) => Ok(Some(value)),
        None => Ok(None),
        _ => Err(super::DriverFactoryError::new(format!(
            "the hosted WebSocketServer `{name}` setting is not bytes"
        ))),
    }
}

#[cfg(feature = "hosted-websocket")]
fn static_text_array(
    spec: &crate::plan::ConnectorSpec,
    name: &str,
) -> Result<Vec<String>, super::DriverFactoryError> {
    match static_value(spec, name)? {
        Some(value) => text_array(value).ok_or_else(|| {
            super::DriverFactoryError::new(format!(
                "the hosted WebSocketServer `{name}` setting is not an array of text"
            ))
        }),
        None => Ok(Vec::new()),
    }
}

#[cfg(feature = "hosted-websocket")]
pub(crate) fn bind_server_routes(
    plan: &ValidatedExecutionPlan,
    nodes: &[crate::node::Path],
) -> Result<Vec<WebSocketServerBinding>, super::DriverFactoryError> {
    let mut routes = Vec::new();
    for path in nodes {
        let spec = plan
            .plan()
            .nodes
            .iter()
            .find(|node| &node.path == path)
            .and_then(|node| match &node.execution {
                NodeExecution::Connector(spec) => Some(spec),
                NodeExecution::Operator => None,
            })
            .ok_or_else(|| {
                super::DriverFactoryError::new(format!(
                    "the WebSocketServer binding path `{path}` is not a connector"
                ))
            })?;
        if spec.contract.as_str() != crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID {
            return Err(super::DriverFactoryError::new(format!(
                "the WebSocketServer binder received `{}` at `{path}`",
                spec.contract
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
                    "the hosted WebSocketServer codec at `{path}` is unresolved"
                )));
            }
        };
        let listen = required_static_text(spec, "listen")?;
        let route_path = required_static_text(spec, "path")?;
        if !route_path.starts_with('/') || route_path.contains(['?', '#']) {
            return Err(super::DriverFactoryError::new(format!(
                "the hosted WebSocketServer path `{route_path}` at `{path}` is not absolute"
            )));
        }
        let max_message = match static_value(spec, "max_message")? {
            Some(Value::Text(value)) => crate::connector::byte_size(&value),
            _ => None,
        }
        .ok_or_else(|| {
            super::DriverFactoryError::new(format!(
                "the hosted WebSocketServer message bound at `{path}` is unresolved"
            ))
        })?;
        let versions = static_text_array(spec, "versions")?;
        if versions.is_empty() {
            return Err(super::DriverFactoryError::new(format!(
                "the hosted WebSocketServer version preference at `{path}` is empty"
            )));
        }
        let typed = static_value(spec, "headers")?.unwrap_or_else(|| Value::Struct(Vec::new()));
        let custom =
            static_value(spec, "custom_headers")?.unwrap_or_else(|| Value::Map(BTreeMap::new()));
        let headers = crate::plan::canonical_request_headers(&typed, &custom)
            .and_then(header_map)
            .ok_or_else(|| {
                super::DriverFactoryError::new(format!(
                    "the hosted WebSocketServer response headers at `{path}` are not canonical"
                ))
            })?;
        routes.push(WebSocketServerBinding {
            allowed_origins: static_text_array(spec, "allowed_origins")?,
            auth_bearer: optional_static_text(spec, "auth_bearer")?,
            codec,
            headers,
            listen,
            max_message,
            node: path.clone(),
            path: route_path,
            subprotocols: static_text_array(spec, "subprotocols")?,
            tls_certificate: optional_static_bytes(spec, "tls_certificate")?,
            tls_private_key: optional_static_bytes(spec, "tls_private_key")?,
            versions,
        });
    }
    Ok(routes)
}

#[cfg(feature = "hosted-websocket")]
fn decode_tagged(value: crate::canonical_cbor::Value) -> Option<Value> {
    use crate::canonical_cbor::Value as Cbor;

    let Cbor::Array(mut tagged) = value else {
        return None;
    };
    if tagged.len() != 2 {
        return None;
    }
    let payload = tagged.pop()?;
    let Cbor::Uint(tag) = tagged.pop()? else {
        return None;
    };
    match (tag, payload) {
        (1, Cbor::Bool(value)) => Some(Value::Bool(value)),
        (7, Cbor::Bytes(value)) => Some(Value::Bytes(value)),
        (8, Cbor::Text(value)) => Some(Value::Text(value)),
        (9, Cbor::Null) => Some(Value::Null),
        (17, Cbor::Array(fields)) => fields
            .into_iter()
            .map(|field| {
                let Cbor::Array(mut pair) = field else {
                    return None;
                };
                if pair.len() != 2 {
                    return None;
                }
                let value = decode_tagged(pair.pop()?)?;
                let Cbor::Text(name) = pair.pop()? else {
                    return None;
                };
                Some((name, value))
            })
            .collect::<Option<Vec<_>>>()
            .map(Value::Struct),
        (18, Cbor::Array(entries)) => entries
            .into_iter()
            .map(|entry| {
                let Cbor::Array(mut pair) = entry else {
                    return None;
                };
                if pair.len() != 2 {
                    return None;
                }
                let value = decode_tagged(pair.pop()?)?;
                let Value::Text(key) = decode_tagged(pair.pop()?)? else {
                    return None;
                };
                Some((key, value))
            })
            .collect::<Option<BTreeMap<_, _>>>()
            .map(Value::Map),
        (19, Cbor::Array(items)) => items
            .into_iter()
            .map(decode_tagged)
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        (20, Cbor::Text(value)) => Some(Value::Enum(value)),
        _ => None,
    }
}

#[cfg(feature = "hosted-websocket")]
fn prepared_value(prepared: &PreparedRequest) -> Option<Value> {
    let (tree, used) = crate::canonical_cbor::decode(prepared.bytes()).ok()?;
    (used == prepared.bytes().len())
        .then(|| decode_tagged(tree))
        .flatten()
}

#[cfg(feature = "hosted-websocket")]
fn optional_text(value: Value) -> Option<Option<String>> {
    match value {
        Value::Null => Some(None),
        Value::Text(value) => Some(Some(value)),
        _ => None,
    }
}

#[cfg(feature = "hosted-websocket")]
fn optional_bytes(value: Value) -> Option<Option<Vec<u8>>> {
    match value {
        Value::Null => Some(None),
        Value::Bytes(value) => Some(Some(value)),
        _ => None,
    }
}

#[cfg(feature = "hosted-websocket")]
fn text_array(value: Value) -> Option<Vec<String>> {
    let Value::Array(values) = value else {
        return None;
    };
    values
        .into_iter()
        .map(|value| match value {
            Value::Text(value) | Value::Enum(value) => Some(value),
            _ => None,
        })
        .collect()
}

#[cfg(feature = "hosted-websocket")]
fn header_map(value: Value) -> Option<BTreeMap<String, Vec<Vec<u8>>>> {
    let Value::Map(entries) = value else {
        return None;
    };
    entries
        .into_iter()
        .map(|(name, value)| {
            let Value::Array(values) = value else {
                return None;
            };
            values
                .into_iter()
                .map(|value| match value {
                    Value::Bytes(value) => Some(value),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .map(|values| (name, values))
        })
        .collect()
}

#[cfg(feature = "hosted-websocket")]
pub(crate) fn decode_message(prepared: &PreparedRequest) -> Option<WebSocketMessage> {
    let Value::Struct(fields) = prepared_value(prepared)? else {
        return None;
    };
    let mut fields: BTreeMap<_, _> = fields.into_iter().collect();
    let Value::Bytes(body) = fields.remove("body")? else {
        return None;
    };
    let Value::Text(codec) = fields.remove("codec")? else {
        return None;
    };
    fields
        .is_empty()
        .then_some(WebSocketMessage { codec, body })
}

#[cfg(feature = "hosted-websocket")]
pub(crate) fn decode_client_open(prepared: &PreparedRequest) -> Option<WebSocketClientOpening> {
    let Value::Struct(fields) = prepared_value(prepared)? else {
        return None;
    };
    let mut fields: BTreeMap<_, _> = fields.into_iter().collect();
    let auth_bearer = optional_text(fields.remove("auth_bearer")?)?;
    let Value::Text(codec) = fields.remove("codec")? else {
        return None;
    };
    let headers = header_map(fields.remove("headers")?)?;
    let Value::Bytes(initial_message) = fields.remove("initial_message")? else {
        return None;
    };
    let subprotocols = text_array(fields.remove("subprotocols")?)?;
    let tls_ca = optional_bytes(fields.remove("tls_ca")?)?;
    let tls_certificate = optional_bytes(fields.remove("tls_certificate")?)?;
    let tls_private_key = optional_bytes(fields.remove("tls_private_key")?)?;
    let Value::Text(url) = fields.remove("url")? else {
        return None;
    };
    let versions = text_array(fields.remove("versions")?)?;
    let Value::Enum(zero_rtt) = fields.remove("zero_rtt")? else {
        return None;
    };
    if zero_rtt != "disabled" || !fields.is_empty() || versions.is_empty() {
        return None;
    }
    Some(WebSocketClientOpening {
        auth_bearer,
        codec,
        headers,
        initial_message,
        subprotocols,
        tls_ca,
        tls_certificate,
        tls_private_key,
        url,
        versions,
    })
}

impl WebSocketLabDriver {
    pub(crate) fn try_bind(
        plan: &ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        connector_id: &ConnectorId,
    ) -> Result<Self, super::DriverFactoryError> {
        let mut specs = BTreeMap::new();
        for path in nodes {
            let spec = plan
                .plan()
                .nodes
                .iter()
                .find(|node| &node.path == path)
                .and_then(|node| match &node.execution {
                    NodeExecution::Connector(spec) => Some(spec),
                    NodeExecution::Operator => None,
                })
                .ok_or_else(|| {
                    super::DriverFactoryError::new(format!(
                        "the WebSocket binding path `{path}` is not a connector"
                    ))
                })?;
            if &spec.contract != connector_id {
                return Err(super::DriverFactoryError::new(format!(
                    "the WebSocket binder for `{connector_id}` received `{}` at `{path}`",
                    spec.contract
                )));
            }
            if !matches!(spec.attribute("codec"), Some(AttrSetting::Word(_))) {
                return Err(super::DriverFactoryError::new(format!(
                    "the WebSocket codec of `{path}` is unresolved"
                )));
            }
            specs.insert(path.clone(), spec.clone());
        }
        Ok(Self { specs })
    }

    pub(crate) fn value(
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

    pub(crate) fn message_bytes(
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

    pub(crate) fn prepare_message(
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

    pub(crate) fn prepare_open(
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
        let typed_headers =
            Self::value(spec, resolved, "headers").unwrap_or_else(|| Value::Struct(Vec::new()));
        let custom_headers = Self::value(spec, resolved, "custom_headers")
            .unwrap_or_else(|| Value::Map(BTreeMap::new()));
        let headers = crate::plan::canonical_request_headers(&typed_headers, &custom_headers)
            .ok_or(crate::boundary::WebSocketExit::ProtocolFailed)?;
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

    #[cfg(feature = "hosted-websocket")]
    pub(crate) fn plan_binding(
        &self,
        connector: &crate::node::Path,
    ) -> Option<WebSocketPlanBinding> {
        let spec = self.specs.get(connector)?;
        let Some(AttrSetting::Word(codec)) = spec.attribute("codec") else {
            return None;
        };
        let max_message = match spec.attribute("max_message") {
            Some(AttrSetting::Value(Value::Text(value))) => crate::connector::byte_size(value)?,
            _ => return None,
        };
        let versions = match spec.attribute("versions") {
            Some(AttrSetting::Value(Value::Array(values))) => values
                .iter()
                .map(|value| match value {
                    Value::Enum(value) | Value::Text(value) => Some(value.clone()),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?,
            _ => return None,
        };
        (!versions.is_empty()).then_some(WebSocketPlanBinding {
            codec: codec.clone(),
            max_message,
            versions,
        })
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

    fn scheduler_session_completion(
        &mut self,
        _: &crate::node::Path,
        completion: crate::boundary::SchedulerCompletion,
    ) -> crate::boundary::Completion {
        crate::boundary::Completion::websocket_exit(match completion {
            crate::boundary::SchedulerCompletion::TimeoutExpired => {
                crate::boundary::WebSocketExit::TimeoutExpired
            }
            crate::boundary::SchedulerCompletion::OutcomeUnknown => {
                crate::boundary::WebSocketExit::OutcomeUnknown
            }
        })
    }

    fn open_session(&mut self, _: &SessionOpenIntent, _: &Envelope) -> SessionStart {
        SessionStart::Started
    }

    fn prepare_session_write(
        &mut self,
        connector: &crate::node::Path,
        _: SessionRef,
        data: &Data,
        _: &BTreeMap<String, crate::value::Value>,
    ) -> RequestPreparation {
        Self::preparation(self.prepare_message(connector, data))
    }

    fn write_session(&mut self, _: &SessionWriteIntent, _: &Envelope) -> SessionStart {
        SessionStart::Started
    }

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
        WebSocketLabDriver::try_bind(plan, nodes, self.connector_id())
            .map(|driver| Box::new(driver) as Box<dyn super::BoundConnectorDriver + 'plan>)
    }
}

pub(crate) fn lab_factory(
    spelling: &str,
) -> Result<Box<dyn super::ConnectorDriverFactory>, super::DriverFactoryError> {
    ConnectorId::parse(spelling)
        .map(|id| Box::new(WebSocketLabFactory { id }) as Box<dyn super::ConnectorDriverFactory>)
        .map_err(|error| super::DriverFactoryError::new(error.to_string()))
}
