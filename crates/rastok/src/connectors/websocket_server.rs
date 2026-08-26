//! WebSocketServer family assembly.
//!
//! Its hosted listener will land here; the effectless H2 twin already binds
//! the exact server contract through the shared WebSocket protocol machine.

pub(crate) fn lab_factory()
-> Result<Box<dyn super::ConnectorDriverFactory>, super::DriverFactoryError> {
    super::websocket_protocol::lab_factory(crate::connector::WEBSOCKET_SERVER_CONNECTOR_ID)
}
