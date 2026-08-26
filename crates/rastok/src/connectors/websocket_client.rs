//! WebSocketClient family assembly.
//!
//! The protocol state machine shared with the server lives in the explicitly
//! named `websocket_protocol` module; this file owns the client contract hook.

pub(crate) fn lab_factory()
-> Result<Box<dyn super::ConnectorDriverFactory>, super::DriverFactoryError> {
    super::websocket_protocol::lab_factory(crate::connector::WEBSOCKET_CLIENT_CONNECTOR_ID)
}
