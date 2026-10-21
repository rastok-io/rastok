//! WebSocketServer family assembly.
//!
//! Its hosted listener will land here; the effectless H2 twin already binds
//! the exact server contract through the shared WebSocket protocol machine.

#[cfg(feature = "hosted-websocket")]
pub use crate::connectors::network::websocket::{
    WebSocketConfig, WebSocketConfigError, WebSocketServerFactory,
};

pub(crate) fn lab_factory() -> Result<Box<dyn super::DriverFactory>, super::DriverFactoryError> {
    super::websocket_protocol::lab_factory(crate::connector::stdlib::WEBSOCKET_SERVER_CONNECTOR_ID)
}
