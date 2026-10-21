//! Hosted Connector Drivers, their factories, runtime association and conformance tests.
//!
//! Physical protocol implementations live here; portable Contract rules and Driver
//! interfaces are shared with `rastok-core` through the parent module.

mod assembly;
pub mod conformance;
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
pub mod registry;
pub mod runtime;
mod websocket_client;
mod websocket_protocol;
mod websocket_server;

pub use assembly::{HostedAssemblyError, hosted_registry};
pub use file::{File, FileBindError, FileFactory, FileSettingsError};
#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
pub use http_client::{HttpClientConfig, HttpClientFactory};
pub(crate) use http_server::HttpServerLabInput;
#[cfg(feature = "hosted-http12")]
pub use http_server::{HttpServerConfig, HttpServerFactory};
pub use manifest::{DriverManifest, DriverManifestEntry, DriverManifestError};
pub(crate) use process::ProcessRequestSettings;
pub use process::{
    Process, ProcessBindError, ProcessFactory, ProcessServiceFactory, ProcessSettingsError,
};
pub use registry::{
    DriverBindingError, DriverFactory, DriverFactoryError, DriverRegistrationError, DriverRegistry,
};
pub use runtime::{InboundPollError, RuntimeDrivers};
#[cfg(feature = "hosted-websocket")]
pub use websocket_client::WebSocketClientFactory;
#[cfg(feature = "hosted-websocket")]
pub use websocket_server::{WebSocketConfig, WebSocketConfigError, WebSocketServerFactory};

/// Request preparation and scheduler completions shared by a physical Call Driver
/// and its Lab substitute.
///
/// These operations do not observe the external world. Process, File and HttpClient
/// each provide the same implementation to their physical Driver and scripted model.
pub(crate) trait CallProtocol {
    fn prepare_request(
        &self,
        connector: &crate::node::Path,
        data: &crate::envelope::Data,
        settings: &std::collections::BTreeMap<String, crate::value::Value>,
    ) -> crate::connector::exchange::RequestPreparation;

    fn scheduler_completion(
        &self,
        completion: crate::connector::exchange::SchedulerCompletion,
    ) -> crate::connector::exchange::Completion;
}

pub(crate) type ScriptedProtocolBinder =
    fn(
        &crate::plan::ValidatedExecutionPlan,
        &[crate::node::Path],
        &crate::deployment::BootBindings,
    ) -> Result<Box<dyn CallProtocol>, registry::DriverFactoryError>;

/// Standard Call protocols available to the Lab, declared beside their physical Drivers.
pub(crate) fn scripted_protocol_binders() -> [(&'static str, ScriptedProtocolBinder); 3] {
    [
        (
            crate::connector::stdlib::PROCESS_CONNECTOR_ID,
            process::bind_scripted_protocol,
        ),
        (
            crate::connector::stdlib::FILE_CONNECTOR_ID,
            file::bind_scripted_protocol,
        ),
        (
            crate::connector::stdlib::HTTP_CLIENT_CONNECTOR_ID,
            http_client::bind_scripted_protocol,
        ),
    ]
}

/// Effectless inbound and Session models, independent of physical transports.
pub(crate) fn inbound_lab_factories(
    input: std::rc::Rc<std::cell::RefCell<HttpServerLabInput>>,
) -> Result<Vec<Box<dyn DriverFactory>>, DriverFactoryError> {
    let mut factories = vec![http_server::lab_factory(input)?];
    factories.push(websocket_client::lab_factory()?);
    factories.push(websocket_server::lab_factory()?);
    Ok(factories)
}
