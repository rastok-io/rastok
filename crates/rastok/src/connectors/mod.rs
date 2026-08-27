//! Runtime implementations of Connector contracts.
//!
//! Each family owns its binding and Driver implementation below this module.
//! The public re-export keeps the pre-M1.5 `connectors::Process` path stable
//! while the catalogue grows without returning to one monolithic file.

mod assembly;
mod file;
mod http_client;
mod http_server;
#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
mod http_shared;
mod manifest;
#[cfg(any(
    feature = "hosted-http12",
    feature = "hosted-http3",
    feature = "hosted-websocket"
))]
mod network;
mod process;
mod registry;
mod websocket_client;
mod websocket_protocol;
mod websocket_server;

pub use assembly::{HostedAssemblyError, hosted_registry};
pub use file::{File, FileBindError, FileFactory, FileSettingsError};
#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
pub use http_client::{HttpClientConfig, HttpClientFactory};
#[cfg(feature = "hosted-http12")]
pub use http_server::{HttpServerConfig, HttpServerFactory};
pub use manifest::{DriverManifest, DriverManifestEntry, DriverManifestError};
#[cfg(test)]
pub(crate) use process::ProcessRequestBindings;
pub use process::{Process, ProcessBindError, ProcessFactory, ProcessSettingsError};
pub use registry::{
    BoundConnectorDriver, ConnectorDriverFactory, DriverBindingError, DriverFactoryError,
    DriverRegistrationError, DriverRegistry, RuntimeBindings,
};
#[cfg(feature = "hosted-websocket")]
pub use websocket_client::WebSocketClientFactory;
#[cfg(feature = "hosted-websocket")]
pub use websocket_server::{WebSocketConfig, WebSocketConfigError, WebSocketServerFactory};

/// The family-owned, effectless half of a scripted Driver.
///
/// Real and Lab Drivers share request framing and scheduler projections; only
/// observation of the external world is replaced. Three shipped families make
/// this a real substitution seam rather than a decorative trait.
pub(crate) trait ScriptedConnectorProtocol {
    fn prepare_request(
        &self,
        connector: &crate::node::Path,
        data: &crate::envelope::Data,
        settings: &std::collections::BTreeMap<String, crate::value::Value>,
    ) -> crate::boundary::RequestPreparation;

    fn scheduler_completion(
        &self,
        completion: crate::boundary::SchedulerCompletion,
    ) -> crate::boundary::Completion;
}

pub(crate) type ScriptedProtocolBinder =
    fn(
        &crate::plan::ValidatedExecutionPlan,
        &[crate::node::Path],
    ) -> Result<Box<dyn ScriptedConnectorProtocol>, registry::DriverFactoryError>;

/// Every built-in family's scripted twin, declared beside its real module.
pub(crate) fn scripted_protocol_binders() -> [(&'static str, ScriptedProtocolBinder); 3] {
    [
        (
            crate::connector::PROCESS_CONNECTOR_ID,
            process::bind_scripted_protocol,
        ),
        (
            crate::connector::FILE_CONNECTOR_ID,
            file::bind_scripted_protocol,
        ),
        (
            crate::connector::HTTP_CLIENT_CONNECTOR_ID,
            http_client::bind_scripted_protocol,
        ),
    ]
}

/// Family-owned effectless Drivers required to bind inbound and session
/// network contracts in the Lab before hosted socket implementations land.
pub(crate) fn passive_lab_factories()
-> Result<Vec<Box<dyn ConnectorDriverFactory>>, DriverFactoryError> {
    let mut factories = vec![http_server::lab_factory()?];
    factories.push(websocket_client::lab_factory()?);
    factories.push(websocket_server::lab_factory()?);
    Ok(factories)
}
