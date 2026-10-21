//! Static hosted Driver composition.
//!
//! Cargo features choose physical protocol stacks. This module turns that
//! compile-time choice into one exact runtime registry; the semantic
//! Connector ids remain unchanged in every build.

use super::{ConnectorDriverFactory, DriverRegistry};
use std::path::Path;

/// Why the selected hosted image cannot construct its Driver registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostedAssemblyError(String);

impl HostedAssemblyError {
    fn at(boundary: &str, error: impl core::fmt::Display) -> Self {
        Self(format!("{boundary}: {error}"))
    }
}

impl core::fmt::Display for HostedAssemblyError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for HostedAssemblyError {}

fn add(
    registry: DriverRegistry,
    factory: impl ConnectorDriverFactory + 'static,
) -> Result<DriverRegistry, HostedAssemblyError> {
    registry
        .try_with(Box::new(factory))
        .map_err(|error| HostedAssemblyError::at("the hosted Driver registry is ambiguous", error))
}

/// Composes every Driver implemented by this static image.
///
/// Process is always present. File is present only when an explicit physical
/// root is supplied. Network families follow Cargo capabilities:
/// `hosted-http12`, `hosted-http3`, `hosted-websocket`, or their
/// `hosted-network` union. A plan requiring an omitted family is rejected by
/// [`super::RuntimeBindings`] before Execution construction.
///
/// # Errors
///
/// Returns the first invalid built-in id, physical File root, configuration,
/// or duplicate registration. No listener, process or network request is
/// started here.
pub fn hosted_registry(file_root: Option<&Path>) -> Result<DriverRegistry, HostedAssemblyError> {
    let process = super::ProcessFactory::try_new()
        .map_err(|error| HostedAssemblyError::at("the Process Driver id is invalid", error))?;
    let mut registry = add(DriverRegistry::default(), process)?;
    registry = add(
        registry,
        super::ProcessServiceFactory::try_new().map_err(|error| {
            HostedAssemblyError::at("the resident Process Driver id is invalid", error)
        })?,
    )?;

    if let Some(root) = file_root {
        let file = super::FileFactory::try_new(root)
            .map_err(|error| HostedAssemblyError::at("the File Driver id is invalid", error))?;
        registry = add(registry, file)?;
    }

    #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
    {
        let client = super::HttpClientFactory::try_new(super::HttpClientConfig::default())
            .map_err(|error| {
                HostedAssemblyError::at("the HttpClient Driver id is invalid", error)
            })?;
        registry = add(registry, client)?;
    }

    #[cfg(feature = "hosted-http12")]
    {
        let server = super::HttpServerFactory::try_new(super::HttpServerConfig::default())
            .map_err(|error| {
                HostedAssemblyError::at("the HttpServer Driver id is invalid", error)
            })?;
        registry = add(registry, server)?;
    }

    #[cfg(feature = "hosted-websocket")]
    {
        let client = super::WebSocketClientFactory::try_new(super::WebSocketConfig::default())
            .map_err(|error| {
                HostedAssemblyError::at("the WebSocketClient Driver id is invalid", error)
            })?;
        registry = add(registry, client)?;
        let server = super::WebSocketServerFactory::try_new(super::WebSocketConfig::default())
            .map_err(|error| {
                HostedAssemblyError::at("the WebSocketServer Driver id is invalid", error)
            })?;
        registry = add(registry, server)?;
    }

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::hosted_registry;

    #[test]
    fn the_manifest_is_the_exact_projection_of_the_static_image() {
        let registry = hosted_registry(None).expect("the built-in ids are canonical");
        let manifest = registry.manifest().expect("registry ids are unique");
        let ids = manifest
            .entries()
            .iter()
            .map(|entry| entry.connector().as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&crate::connector::PROCESS_CONNECTOR_ID));
        assert!(!ids.contains(&crate::connector::FILE_CONNECTOR_ID));
        #[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
        assert!(ids.contains(&crate::connector::HTTP_CLIENT_CONNECTOR_ID));
        #[cfg(feature = "hosted-http12")]
        assert!(ids.contains(&crate::connector::HTTP_SERVER_CONNECTOR_ID));
        #[cfg(feature = "hosted-websocket")]
        {
            assert!(ids.contains(&crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID));
            assert!(ids.contains(&crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID));
        }
    }

    #[cfg(feature = "hosted-network")]
    #[test]
    fn the_full_hosted_image_composes_all_six_families_once() {
        let registry = hosted_registry(Some(std::path::Path::new("unused-file-root")))
            .expect("the full static image composes");
        let manifest = registry.manifest().expect("registry ids are unique");
        let ids = manifest
            .entries()
            .iter()
            .map(|entry| entry.connector().as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                crate::connector::FILE_CONNECTOR_ID,
                crate::connector::HTTP_CLIENT_CONNECTOR_ID,
                crate::connector::HTTP_SERVER_CONNECTOR_ID,
                crate::connector::PROCESS_CONNECTOR_ID,
                crate::connector::PROCESS_SERVICE_CONNECTOR_ID,
                crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID,
                crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID,
            ]
        );
    }
}
