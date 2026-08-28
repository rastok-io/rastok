//! HttpClient contract binding shared by the Lab and hosted Driver.
//!
//! The outbound request descriptor is already family-owned and deterministic:
//! mocks and the real Driver agree on the exact method, URL,
//! headers and body covered by the request digest. The server twin is a
//! deliberately passive inbound Driver; scenario injections enter through
//! the ordinary Lab boundary, never through a fake socket.

use crate::boundary::{
    Completion, HttpClientExit, PreparedRequest, RequestPreparation, SchedulerCompletion,
};
use crate::envelope::Data;
use crate::plan::{AttrSetting, NodeExecution, ValidatedExecutionPlan};
use crate::value::Value;
use std::collections::BTreeMap;

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
pub use crate::connectors::network::http_client::{HttpClientConfig, HttpClientFactory};

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
use super::http_shared::HttpVersion;

/// The exact request descriptor recovered from [`PreparedRequest`] bytes.
///
/// This is a private semantic type, not a second framing. Decoding is the
/// inverse of [`HttpClientRequestBindings::prepare`], so the physical engine
/// executes the already-journaled bytes instead of rebuilding them from an
/// envelope or settings.
#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpRequest {
    pub(crate) auth_bearer: Option<String>,
    pub(crate) body: Vec<u8>,
    pub(crate) codec: String,
    pub(crate) headers: BTreeMap<String, Vec<Vec<u8>>>,
    pub(crate) idempotency_key: Option<String>,
    pub(crate) method: String,
    pub(crate) url: String,
}

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpClientPlanBinding {
    pub(crate) max_body: u64,
    pub(crate) versions: Vec<HttpVersion>,
}

#[derive(Debug)]
pub(crate) struct HttpClientRequestBindings {
    pub(crate) specs: BTreeMap<crate::node::Path, crate::plan::ConnectorSpec>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HttpRequestRejection {
    DecodeFailed,
    BodyTooLarge { limit: u64 },
}

impl HttpRequestRejection {
    pub(crate) fn completion(self) -> Completion {
        Completion::http_client_exit(match self {
            Self::DecodeFailed => HttpClientExit::DecodeFailed,
            Self::BodyTooLarge { limit } => HttpClientExit::BodyTooLarge { limit },
        })
    }
}

impl HttpClientRequestBindings {
    pub(crate) fn try_bind(
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
                .and_then(|node| match &node.execution {
                    NodeExecution::Connector(spec) => Some(spec),
                    NodeExecution::Operator => None,
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

    fn text(
        spec: &crate::plan::ConnectorSpec,
        resolved: &BTreeMap<String, Value>,
        name: &str,
    ) -> Result<String, HttpRequestRejection> {
        match Self::value(spec, resolved, name) {
            Some(Value::Text(value)) => Ok(value),
            _ => Err(HttpRequestRejection::DecodeFailed),
        }
    }

    pub(crate) fn prepare(
        &self,
        connector: &crate::node::Path,
        data: &Data,
        resolved: &BTreeMap<String, Value>,
    ) -> Result<PreparedRequest, HttpRequestRejection> {
        let spec = self
            .specs
            .get(connector)
            .ok_or(HttpRequestRejection::DecodeFailed)?;
        let codec = match spec.attribute("codec") {
            Some(AttrSetting::Word(codec)) => codec.as_str(),
            _ => return Err(HttpRequestRejection::DecodeFailed),
        };
        let body = crate::boundary::network_payload_bytes(codec, data)
            .map_err(|_| HttpRequestRejection::DecodeFailed)?;
        let max_body = match spec.attribute("max_body") {
            Some(AttrSetting::Value(Value::Text(value))) => {
                crate::connector::byte_size(value).ok_or(HttpRequestRejection::DecodeFailed)?
            }
            _ => return Err(HttpRequestRejection::DecodeFailed),
        };
        if u64::try_from(body.len()).map_or(true, |length| length > max_body) {
            return Err(HttpRequestRejection::BodyTooLarge { limit: max_body });
        }
        let typed_headers =
            Self::value(spec, resolved, "headers").unwrap_or_else(|| Value::Struct(Vec::new()));
        let custom_headers = Self::value(spec, resolved, "custom_headers")
            .unwrap_or_else(|| Value::Map(BTreeMap::new()));
        let headers = crate::plan::canonical_request_headers(&typed_headers, &custom_headers)
            .ok_or(HttpRequestRejection::DecodeFailed)?;
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

    #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
    pub(crate) fn plan_binding(
        &self,
        connector: &crate::node::Path,
    ) -> Option<HttpClientPlanBinding> {
        let spec = self.specs.get(connector)?;
        let max_body = match spec.attribute("max_body") {
            Some(AttrSetting::Value(Value::Text(value))) => crate::connector::byte_size(value)?,
            _ => return None,
        };
        let versions = match spec.attribute("versions") {
            Some(AttrSetting::Value(Value::Array(values))) => values
                .iter()
                .map(|value| match value {
                    Value::Enum(value) | Value::Text(value) => HttpVersion::parse(value),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?,
            _ => return None,
        };
        (!versions.is_empty()).then_some(HttpClientPlanBinding { max_body, versions })
    }
}

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
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
        _ => None,
    }
}

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
pub(crate) fn decode_request(prepared: &PreparedRequest) -> Option<HttpRequest> {
    let (tree, used) = crate::canonical_cbor::decode(prepared.bytes()).ok()?;
    if used != prepared.bytes().len() {
        return None;
    }
    let Value::Struct(fields) = decode_tagged(tree)? else {
        return None;
    };
    let mut fields: BTreeMap<_, _> = fields.into_iter().collect();
    let optional_text = |value: Value| match value {
        Value::Null => Some(None),
        Value::Text(value) => Some(Some(value)),
        _ => None,
    };
    let auth_bearer = optional_text(fields.remove("auth_bearer")?)?;
    let Value::Bytes(body) = fields.remove("body")? else {
        return None;
    };
    let Value::Text(codec) = fields.remove("codec")? else {
        return None;
    };
    let Value::Map(header_values) = fields.remove("headers")? else {
        return None;
    };
    let headers = header_values
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
        .collect::<Option<BTreeMap<_, _>>>()?;
    let idempotency_key = optional_text(fields.remove("idempotency_key")?)?;
    let Value::Text(method) = fields.remove("method")? else {
        return None;
    };
    let Value::Text(url) = fields.remove("url")? else {
        return None;
    };
    fields.is_empty().then_some(HttpRequest {
        auth_bearer,
        body,
        codec,
        headers,
        idempotency_key,
        method,
        url,
    })
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
            Err(rejection) => RequestPreparation::Rejected(rejection.completion()),
        }
    }

    fn scheduler_completion(&self, completion: SchedulerCompletion) -> Completion {
        Completion::http_client_exit(match completion {
            SchedulerCompletion::TimeoutExpired => HttpClientExit::TimeoutExpired,
            SchedulerCompletion::OutcomeUnknown => HttpClientExit::OutcomeUnknown,
        })
    }
}

pub(crate) fn bind_scripted_protocol(
    plan: &ValidatedExecutionPlan,
    nodes: &[crate::node::Path],
) -> Result<Box<dyn super::ScriptedConnectorProtocol>, super::DriverFactoryError> {
    HttpClientRequestBindings::try_bind(plan, nodes)
        .map(|bindings| Box::new(bindings) as Box<dyn super::ScriptedConnectorProtocol>)
}
