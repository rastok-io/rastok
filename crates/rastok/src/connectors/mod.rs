//! Runtime implementations of Connector contracts.
//!
//! Each family owns its binding and Driver implementation below this module.
//! The public re-export keeps the pre-M1.5 `connectors::Process` path stable
//! while the catalogue grows without returning to one monolithic file.

mod file;
mod manifest;
mod process;
mod registry;

pub use file::{File, FileBindError, FileFactory, FileSettingsError};
pub use manifest::{DriverManifest, DriverManifestEntry, DriverManifestError};
#[cfg(test)]
pub(crate) use process::ProcessRequestBindings;
pub use process::{Process, ProcessBindError, ProcessFactory, ProcessSettingsError};
pub use registry::{
    BoundConnectorDriver, ConnectorDriverFactory, DriverBindingError, DriverFactoryError,
    DriverRegistrationError, DriverRegistry, RuntimeBindings,
};

/// The family-owned, effectless half of a scripted Driver.
///
/// Real and Lab Drivers share request framing and scheduler projections; only
/// observation of the external world is replaced. Two shipped families make
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
pub(crate) fn scripted_protocol_binders() -> [(&'static str, ScriptedProtocolBinder); 2] {
    [
        (
            crate::connector::PROCESS_CONNECTOR_ID,
            process::bind_scripted_protocol,
        ),
        (
            crate::connector::FILE_CONNECTOR_ID,
            file::bind_scripted_protocol,
        ),
    ]
}
