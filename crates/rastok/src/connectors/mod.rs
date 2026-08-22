//! Runtime implementations of Connector contracts.
//!
//! Each family owns its binding and Driver implementation below this module.
//! The public re-export keeps the pre-M1.5 `connectors::Process` path stable
//! while the catalogue grows without returning to one monolithic file.

mod process;
mod registry;

pub use process::{Process, ProcessBindError, ProcessFactory, ProcessSettingsError};
pub use registry::{
    BoundConnectorDriver, ConnectorDriverFactory, DriverBindingError, DriverFactoryError,
    DriverRegistrationError, DriverRegistry, RuntimeBindings,
};
