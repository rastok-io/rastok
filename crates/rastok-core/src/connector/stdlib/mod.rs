//! Portable standard Connector Contracts and their protocol values.
//!
//! These rules are independent of physical Drivers and are available to no_std
//! plan validation as well as the hosted compiler and Lab.

use crate::connector::contract::{ConnectorContract, PortDataType, PortTypeChoice};
use crate::connector::exchange::PreparedRequest;
use crate::datatype::{DataType, StructType, TaggedCase, TypeError};
use crate::envelope;
use crate::node;
use crate::plan::{ConnectorSpec, Node, PlanValidationError};
use alloc::{string::String, vec::Vec};

pub mod process;
pub use process::{
    OutputOverflowReason, ProcessExit, prepare_process_request, process_attribute_site,
    process_exit_type, process_port_type, process_scheduler_completion,
};

pub mod file;
pub use file::{
    FileExit, file_attribute_site, file_exit_type, file_port_type, file_scheduler_completion,
};

pub mod http_client;
pub use http_client::{HttpClientExit, http_client_exit_type};

pub mod http_server;
pub use http_server::{HttpServerExit, http_server_exit_type};

pub mod websocket;
pub use websocket::{WebSocketExit, websocket_exit_type};

/// The request-framing codec of a connector — read once from the
/// spec, dispatched here (D-2026-08-11-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestCodec {
    /// Text lines — opaque payloads pass through.
    Lines,
    /// One JSON value per line, the JSON model exactly.
    Jsonl,
    /// One opaque byte payload.
    Bytes,
}

/// Registry-free effective port type for every M1.6 network Contract.
#[must_use]
pub fn network_port_type(
    id: &crate::connector::ContractId,
    port: &str,
    codec: Option<&str>,
) -> Option<DataType> {
    let definition = contract(id).filter(|definition| {
        matches!(
            definition.id,
            HTTP_CLIENT_CONNECTOR_ID
                | HTTP_SERVER_CONNECTOR_ID
                | WEBSOCKET_CLIENT_CONNECTOR_ID
                | WEBSOCKET_SERVER_CONNECTOR_ID
        )
    })?;
    definition
        .port_type(port, |attribute| {
            (attribute == "codec").then_some(codec).flatten()
        })
        .ok()
}

/// Registry-free evaluation sites for the four M1.6 network Contracts.
#[must_use]
pub fn network_attribute_site(
    id: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    let definition = contract(id).filter(|definition| {
        matches!(
            definition.id,
            HTTP_CLIENT_CONNECTOR_ID
                | HTTP_SERVER_CONNECTOR_ID
                | WEBSOCKET_CLIENT_CONNECTOR_ID
                | WEBSOCKET_SERVER_CONNECTOR_ID
        )
    })?;
    definition_attribute_site(definition, attribute)
}

/// Registry-free evaluation sites for every built-in Connector.
#[must_use]
pub fn connector_attribute_site(
    id: &crate::connector::ContractId,
    attribute: &str,
) -> Option<crate::plan::EvaluationSite> {
    if matches!(
        id.as_str(),
        PROCESS_CONNECTOR_ID | PROCESS_SERVICE_CONNECTOR_ID
    ) {
        return process_attribute_site(id, attribute);
    }
    definition_attribute_site(contract(id)?, attribute)
}

/// Which deployment reference forms one Connector setting explicitly admits.
/// The table is Registry-free so a decoded package cannot turn a later-phase
/// expression into an undeclared boot capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootReferencePolicy {
    /// A typed non-secret value may come from `env("NAME")`.
    Environment,
    /// Material may come only from `secret("logical/path")`.
    Secret,
}

impl BootReferencePolicy {
    /// Whether this declared policy admits one exact reference kind.
    #[must_use]
    pub const fn admits(self, reference: &crate::deployment::BootReference) -> bool {
        matches!(
            (self, reference),
            (
                Self::Environment,
                crate::deployment::BootReference::Environment(_)
            ) | (Self::Secret, crate::deployment::BootReference::Secret(_))
        )
    }
}

/// Registry-free boot-reference policy for every currently executable
/// Connector setting. Absence means that `env()` and `secret()` both refuse.
#[must_use]
pub fn connector_boot_reference_policy(
    contract: &crate::connector::ContractId,
    attribute: &str,
) -> Option<BootReferencePolicy> {
    use BootReferencePolicy::{Environment, Secret};

    match contract.as_str() {
        crate::connector::stdlib::HTTP_CLIENT_CONNECTOR_ID => match attribute {
            // A4 intentionally opens the smallest executable surface. Each
            // later setting joins this table only with its real and scripted
            // Driver consumers in the same gate.
            "auth_bearer" => Some(Secret),
            "url" => Some(Environment),
            _ => None,
        },
        _ => None,
    }
}

fn protocol_code_type() -> DataType {
    // The source expression language's ordinary integer is the portable
    // carrier. Drivers prove the protocol-specific u16 range at ingress.
    DataType::INT
}

fn empty_case(name: &str) -> Result<TaggedCase, TypeError> {
    TaggedCase::new(name, StructType::new(Vec::new())?)
}

fn tagged_value(case: &str, payload: Vec<(String, crate::value::Value)>) -> crate::value::Value {
    crate::value::Value::TaggedUnion(alloc::boxed::Box::new(crate::value::TaggedUnionValue {
        case: String::from(case),
        payload,
    }))
}

/// Frames one request under the given codec.
///
/// # Errors
/// The typed decode refusal, when the payload has no frame under
/// this codec.
pub fn prepare_request(
    codec: RequestCodec,
    data: &envelope::Data,
) -> Result<PreparedRequest, String> {
    match codec {
        RequestCodec::Lines => prepare_lines(data),
        RequestCodec::Jsonl => prepare_jsonl(data),
        RequestCodec::Bytes => prepare_bytes(data),
    }
}

/// Frames one HTTP body or complete WebSocket message without transport
/// delimiters. HTTP versions, WebSocket fragmentation and frame headers are
/// physical Driver concerns; this function owns only the sheet's logical
/// `json | text | bytes` codec.
///
/// # Errors
///
/// The payload does not inhabit the selected logical codec.
pub fn network_payload_bytes(codec: &str, data: &envelope::Data) -> Result<Vec<u8>, String> {
    use envelope::{Data, Value};
    match (codec, data) {
        ("bytes", _) => Ok(bytes_payload(data)?.to_vec()),
        ("text", Data::Materialized(Value::Text(text))) => Ok(text.as_bytes().to_vec()),
        ("text", Data::Encoded { encoding, .. }) => Err(alloc::format!(
            "the text codec requires materialized text; the payload is {encoding:?}-encoded"
        )),
        ("text", Data::Materialized(other)) => Err(alloc::format!(
            "the text codec requires text; the payload is {:?}",
            crate::value::ValueKind::of(other)
        )),
        ("json", Data::Materialized(value)) => crate::json::write_value(value)
            .map(String::into_bytes)
            .map_err(|error| alloc::format!("the payload has no JSON form: {}", error.message)),
        ("json", Data::Encoded { encoding, .. }) => Err(alloc::format!(
            "the json codec frames the JSON model; the payload is {encoding:?}-encoded"
        )),
        (unknown, _) => Err(alloc::format!("unknown network body codec `{unknown}`")),
    }
}

/// Frames one opaque byte request without text or data-model conversion.
///
/// # Errors
///
/// The typed refusal when the payload is neither raw encoded bytes nor the
/// materialized bytes scalar.
pub fn prepare_bytes(data: &envelope::Data) -> Result<PreparedRequest, String> {
    Ok(PreparedRequest::seal(bytes_payload(data)?.to_vec()))
}

/// Borrows one opaque byte payload without framing or sealing it.
///
/// Connector-specific operation framers use this seam when the raw payload is
/// only one field of their final [`PreparedRequest`]. It avoids constructing
/// and hashing a discarded intermediate request.
///
/// # Errors
///
/// The typed refusal when the payload is neither raw encoded bytes nor the
/// materialized bytes scalar.
pub fn bytes_payload(data: &envelope::Data) -> Result<&[u8], String> {
    use envelope::{Data, Encoding, Value};
    match data {
        Data::Encoded {
            encoding: Encoding::Raw,
            bytes,
            ..
        } => Ok(bytes),
        Data::Materialized(Value::Bytes(bytes)) => Ok(bytes),
        Data::Encoded { encoding, .. } => Err(alloc::format!(
            "the bytes codec requires raw bytes; the request is {encoding:?}-encoded"
        )),
        Data::Materialized(other) => Err(alloc::format!(
            "the bytes codec requires bytes; the request data is {:?}",
            crate::value::ValueKind::of(other)
        )),
    }
}

/// Frames a request as ONE canonical JSON line (D-2026-08-11-1):
/// the writer's bytes are `request_digest` material. Encoded
/// payloads and byte scalars refuse typed — `jsonl` frames the JSON
/// model, not opaque representations.
///
/// # Errors
/// The typed decode refusal.
pub fn prepare_jsonl(data: &envelope::Data) -> Result<PreparedRequest, String> {
    use envelope::Data;
    let value = match data {
        Data::Materialized(value) => value,
        Data::Encoded { encoding, .. } => {
            return Err(alloc::format!(
                "the jsonl codec frames the JSON model; the request is {encoding:?}-encoded"
            ));
        }
    };
    let mut line = crate::json::write_value(value)
        .map_err(|error| alloc::format!("the request has no JSON form: {}", error.message))?;
    line.push('\n');
    Ok(PreparedRequest::seal(line.into_bytes()))
}

/// Prepares the plain-text request wire: raw bytes or the textual
/// scalar's UTF-8 form, exactly as before the codec split.
pub fn prepare_lines(data: &envelope::Data) -> Result<PreparedRequest, String> {
    use envelope::{Data, Encoding, Value};
    let bytes = match data {
        Data::Encoded {
            encoding: Encoding::Raw,
            bytes,
            ..
        } => bytes.to_vec(),
        Data::Materialized(Value::Text(text)) => {
            let mut bytes = text.clone().into_bytes();
            if bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            bytes
        }
        Data::Encoded { encoding, .. } => {
            return Err(alloc::format!(
                "the lines codec frames text; the request is {encoding:?}-encoded"
            ));
        }
        Data::Materialized(other) => {
            return Err(alloc::format!(
                "the lines codec frames text; the request data is {:?}",
                crate::value::ValueKind::of(other)
            ));
        }
    };
    Ok(PreparedRequest::seal(bytes))
}

/// The standard Process contract's stable major-version identity.
pub const PROCESS_CONNECTOR_ID: &str = "std/process@4";

/// Resident stdio Session, distinct from the finite job Contract.
pub const PROCESS_SERVICE_CONNECTOR_ID: &str = "std/process@5";

/// The standard single-file contract's stable major-version identity.
pub const FILE_CONNECTOR_ID: &str = "std/file@3";

/// The standard outbound HTTP call contract's stable identity.
pub const HTTP_CLIENT_CONNECTOR_ID: &str = "std/http-client@3";

/// The standard inbound HTTP request contract's stable identity.
pub const HTTP_SERVER_CONNECTOR_ID: &str = "std/http-server@3";

/// The standard outbound WebSocket session contract's stable identity.
pub const WEBSOCKET_CLIENT_CONNECTOR_ID: &str = "std/websocket-client@3";

/// The standard accepted WebSocket session contract's stable identity.
pub const WEBSOCKET_SERVER_CONNECTOR_ID: &str = "std/websocket-server@3";

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

pub use file::FILE;
pub use http_client::HTTP_CLIENT;
pub use http_server::HTTP_SERVER;
pub use process::PROCESS;
pub mod websocket_client;
pub use websocket_client::WEBSOCKET_CLIENT;
pub mod websocket_server;
pub use websocket_server::WEBSOCKET_SERVER;

/// Standard Contract definitions, in a fixed catalogue order.
pub const CONTRACTS: &[&ConnectorContract] = &[
    &PROCESS,
    &FILE,
    &HTTP_CLIENT,
    &HTTP_SERVER,
    &WEBSOCKET_CLIENT,
    &WEBSOCKET_SERVER,
];

/// Finds the standard definition for an exact Contract identity.
#[must_use]
pub fn contract(id: &crate::connector::ContractId) -> Option<&'static ConnectorContract> {
    CONTRACTS
        .iter()
        .copied()
        .find(|definition| definition.protocol_ids().any(|known| known == id.as_str()))
}

fn effective_contract_port(
    node: &Node,
    loop_spec: Option<&crate::plan::LoopSpec>,
    name: &str,
    base: Option<DataType>,
) -> Option<DataType> {
    let Some(spec) = loop_spec.filter(|spec| {
        spec.expired
            .as_ref()
            .is_some_and(|expiry| expiry.port.name() == name)
    }) else {
        return base;
    };
    let DataType::TaggedUnion(union) = base? else {
        return None;
    };
    let state = node
        .ports
        .iter()
        .find(|port| port.port == spec.entry)?
        .ty
        .clone()?;
    node::loop_call_exit_type(&union, state).ok()
}

mod network;

/// Independently checks standard Contracts without a hosted registry or Driver.
pub(crate) fn validate(
    plan: &crate::plan::ExecutionPlan,
    index: crate::node::NodeIndex,
    node: &Node,
    spec: &ConnectorSpec,
    receives_inbound_call_response: bool,
) -> Result<(), PlanValidationError> {
    http_server::validate_response_path(plan, index, node, spec, receives_inbound_call_response)?;
    match spec.contract.as_str() {
        PROCESS_CONNECTOR_ID | PROCESS_SERVICE_CONNECTOR_ID => process::validate(node, spec),
        FILE_CONNECTOR_ID => file::validate(node, spec),
        _ => network::validate(node, spec, node.loop_spec()).map(|_| ()),
    }
}

fn definition_attribute_site(
    definition: &ConnectorContract,
    name: &str,
) -> Option<crate::plan::EvaluationSite> {
    if definition
        .selector
        .as_ref()
        .is_some_and(|selector| selector.attribute == name)
    {
        return Some(crate::plan::EvaluationSite::Plan);
    }
    // Phase lookup describes an attribute independently of mode selection.
    // The Contract validator separately checks whether that mode admits it.
    definition
        .common
        .iter()
        .chain(
            definition
                .selector
                .iter()
                .flat_map(|selector| selector.profiles)
                .flat_map(|mode| mode.attributes),
        )
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.latest_site)
}
