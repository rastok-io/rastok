//! HttpClient contract binding shared by the Lab and hosted Driver.
//!
//! The outbound request descriptor is already family-owned and deterministic:
//! mocks and the real Driver agree on the exact method, URL, ordinary headers
//! and body covered by the request digest. An attested bearer remains a
//! logical `SecretName` in that descriptor; only the real Driver overlays its
//! material. The server twin is a deliberately passive inbound Driver;
//! scenario injections enter through the ordinary Lab boundary, never through
//! a fake socket.

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
/// inverse of [`HttpClientRequestSettings::prepare`], so the physical engine
/// executes the already-journaled descriptor instead of rebuilding it from
/// an envelope or settings. A secret-bearing authorization field retains only
/// its logical reference until the authorized Driver applies the attested
/// material.
#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum HttpBearer {
    Inline(String),
    Secret(crate::deployment::SecretName),
}

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
impl core::fmt::Debug for HttpBearer {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Inline(_) => formatter.write_str("Inline([REDACTED])"),
            Self::Secret(reference) => formatter.debug_tuple("Secret").field(reference).finish(),
        }
    }
}

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HttpRequest {
    pub(crate) auth_bearer: Option<HttpBearer>,
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
    pub(crate) response_type: Option<crate::datatype::DataType>,
    pub(crate) json_profile: rastok_core::json::HydrationProfile,
}

#[derive(Debug)]
pub(crate) struct HttpClientRequestSettings {
    pub(crate) specs: BTreeMap<crate::node::Path, crate::plan::ConnectorSpec>,
    boot_environment: BTreeMap<(crate::node::Path, String), Value>,
    boot_secrets: BTreeMap<(crate::node::Path, String), crate::deployment::SecretName>,
    #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
    response_types: BTreeMap<crate::node::Path, crate::datatype::DataType>,
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

impl HttpClientRequestSettings {
    pub(crate) fn try_bind_with_boot(
        plan: &ValidatedExecutionPlan,
        nodes: &[crate::node::Path],
        boot: &crate::deployment::BootBindings,
    ) -> Result<Self, super::DriverFactoryError> {
        let mut specs = BTreeMap::new();
        let mut boot_environment = BTreeMap::new();
        let mut boot_secrets = BTreeMap::new();
        #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
        let mut response_types = BTreeMap::new();
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
            #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
            if let Some(ty) = plan
                .plan()
                .nodes
                .iter()
                .find(|node| &node.path == path)
                .and_then(|node| node.ports.iter().find(|port| port.port.name() == "out"))
                .and_then(|port| port.ty.as_ref())
                .filter(|ty| **ty != crate::datatype::DataType::Variant)
                && matches!(spec.attribute("codec"), Some(AttrSetting::Word(word)) if word == "json")
            {
                response_types.insert(path.clone(), ty.clone());
            }
            for (name, setting) in &spec.attributes {
                let AttrSetting::Expr { site, expr } = setting else {
                    continue;
                };
                let reference = crate::expr::boot_reference(&expr.root)
                    .map_err(|error| super::DriverFactoryError::new(error.to_string()))?;
                let Some(reference) = reference else {
                    continue;
                };
                if *site != crate::plan::EvaluationSite::Boot
                    || !crate::boundary::connector_boot_reference_policy(&spec.contract, name)
                        .is_some_and(|policy| policy.admits(&reference))
                {
                    return Err(super::DriverFactoryError::new(format!(
                        "the HttpClient setting `{name}` at `{path}` is not boot-compatible"
                    )));
                }
                match reference {
                    crate::deployment::BootReference::Environment(environment) => {
                        let value = boot.environment(&environment).ok_or_else(|| {
                            super::DriverFactoryError::new(format!(
                                "deployment value `{}` required by `{path}.{name}` is missing",
                                environment.as_str()
                            ))
                        })?;
                        if let Err(violation) = expr.ty.check_value_type(name, value) {
                            return Err(super::DriverFactoryError::new(format!(
                                "deployment value `{}` does not fit `{path}.{name}`: {violation}",
                                environment.as_str()
                            )));
                        }
                        boot_environment.insert((path.clone(), name.clone()), value.clone());
                    }
                    crate::deployment::BootReference::Secret(reference) => {
                        if boot.secret(&reference).is_none() {
                            return Err(super::DriverFactoryError::new(format!(
                                "secret `{}` required by `{path}.{name}` is missing",
                                reference.as_str()
                            )));
                        }
                        boot_secrets.insert((path.clone(), name.clone()), reference);
                    }
                }
            }
            specs.insert(path.clone(), spec.clone());
        }
        Ok(Self {
            specs,
            boot_environment,
            boot_secrets,
            #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
            response_types,
        })
    }

    pub(crate) fn value(
        &self,
        connector: &crate::node::Path,
        spec: &crate::plan::ConnectorSpec,
        resolved: &BTreeMap<String, Value>,
        name: &str,
    ) -> Option<Value> {
        match spec.attribute(name) {
            Some(AttrSetting::Word(word)) => Some(Value::Enum(word.clone())),
            Some(AttrSetting::Value(value)) => Some(value.clone()),
            Some(AttrSetting::Expr {
                site: crate::plan::EvaluationSite::Boot,
                ..
            }) => self
                .boot_environment
                .get(&(connector.clone(), name.to_string()))
                .cloned(),
            Some(AttrSetting::Expr { .. }) => resolved.get(name).cloned(),
            None => None,
        }
    }

    #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
    pub(crate) fn secret_references(&self) -> impl Iterator<Item = &crate::deployment::SecretName> {
        self.boot_secrets.values()
    }

    fn secret_reference(
        &self,
        connector: &crate::node::Path,
        name: &str,
    ) -> Option<&crate::deployment::SecretName> {
        self.boot_secrets
            .get(&(connector.clone(), name.to_string()))
    }

    fn text(
        &self,
        connector: &crate::node::Path,
        spec: &crate::plan::ConnectorSpec,
        resolved: &BTreeMap<String, Value>,
        name: &str,
    ) -> Result<String, HttpRequestRejection> {
        match self.value(connector, spec, resolved, name) {
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
        let typed_headers = self
            .value(connector, spec, resolved, "headers")
            .unwrap_or_else(|| Value::Struct(Vec::new()));
        let custom_headers = self
            .value(connector, spec, resolved, "custom_headers")
            .unwrap_or_else(|| Value::Map(BTreeMap::new()));
        let headers = crate::plan::canonical_request_headers(&typed_headers, &custom_headers)
            .ok_or(HttpRequestRejection::DecodeFailed)?;
        let request = Value::Struct(vec![
            (
                String::from("auth_bearer"),
                self.secret_reference(connector, "auth_bearer").map_or_else(
                    || {
                        self.value(connector, spec, resolved, "auth_bearer")
                            .unwrap_or(Value::Null)
                    },
                    |reference| {
                        Value::Struct(vec![(
                            String::from("secret_ref"),
                            Value::Text(reference.as_str().to_string()),
                        )])
                    },
                ),
            ),
            (String::from("body"), Value::Bytes(body)),
            (String::from("codec"), Value::Text(codec.to_string())),
            (String::from("headers"), headers),
            (
                String::from("idempotency_key"),
                self.value(connector, spec, resolved, "idempotency_key")
                    .unwrap_or(Value::Null),
            ),
            (
                String::from("method"),
                Value::Text(self.text(connector, spec, resolved, "method")?),
            ),
            (
                String::from("url"),
                Value::Text(self.text(connector, spec, resolved, "url")?),
            ),
        ]);
        let bytes = crate::canonical_cbor::encode(&crate::canonical::value(&request));
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
        (!versions.is_empty()).then_some(HttpClientPlanBinding {
            max_body,
            versions,
            response_type: self.response_types.get(connector).cloned(),
            json_profile: match spec.attribute("json_profile") {
                None => rastok_core::json::HydrationProfile::Strict,
                Some(AttrSetting::Word(word)) if word == "projected" => {
                    rastok_core::json::HydrationProfile::Projected
                }
                Some(_) => return None,
            },
        })
    }
}

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
pub(crate) fn decode_request(prepared: &PreparedRequest) -> Option<HttpRequest> {
    let Value::Struct(fields) = crate::canonical::decode_data_value(prepared.bytes()).ok()? else {
        return None;
    };
    let mut fields: BTreeMap<_, _> = fields.into_iter().collect();
    let auth_bearer = match fields.remove("auth_bearer")? {
        Value::Null => None,
        Value::Text(value) => Some(HttpBearer::Inline(value)),
        Value::Struct(mut fields) if fields.len() == 1 && fields[0].0 == "secret_ref" => {
            let (_, Value::Text(reference)) = fields.pop()? else {
                return None;
            };
            Some(HttpBearer::Secret(
                crate::deployment::SecretName::new(reference).ok()?,
            ))
        }
        _ => return None,
    };
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
    let idempotency_key = match fields.remove("idempotency_key")? {
        Value::Null => None,
        Value::Text(value) => Some(value),
        _ => return None,
    };
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

impl super::ScriptedConnectorProtocol for HttpClientRequestSettings {
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
    boot: &crate::deployment::BootBindings,
) -> Result<Box<dyn super::ScriptedConnectorProtocol>, super::DriverFactoryError> {
    HttpClientRequestSettings::try_bind_with_boot(plan, nodes, boot)
        .map(|bindings| Box::new(bindings) as Box<dyn super::ScriptedConnectorProtocol>)
}
