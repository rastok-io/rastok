//! WebSocketClient family assembly.
//!
//! The protocol state machine shared with the server lives in the explicitly
//! named `websocket_protocol` module; this file owns the client contract hook.

#[cfg(feature = "hosted-websocket")]
pub use crate::connector::drivers::network::websocket::WebSocketClientFactory;

pub(crate) fn lab_factory() -> Result<Box<dyn super::DriverFactory>, super::DriverFactoryError> {
    super::websocket_protocol::lab_factory(crate::connector::stdlib::WEBSOCKET_CLIENT_CONNECTOR_ID)
}
