//! Hosted socket machinery shared by the network Connector families.
//!
//! This module is deliberately private: protocol-library and async-runtime
//! types stop here. The synchronous Driver boundary exchanges only Rastok
//! facts with the VM.

// H3 lands the physical seam, H4 attaches HttpClient and H5.1 attaches
// HttpServer ack over H1-H2. Keeping the remaining H5-H6 session machinery
// private avoids turning staged code into public API merely to silence lints.
#[allow(dead_code)]
pub(super) mod pump;

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
pub(crate) mod http_client;

#[cfg(feature = "hosted-http12")]
pub(crate) mod http_server;

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
#[allow(dead_code)]
pub(super) mod tls;

#[cfg(feature = "hosted-websocket")]
#[allow(dead_code)]
mod websocket_codec;

#[cfg(all(
    test,
    feature = "hosted-http12",
    feature = "hosted-http3",
    feature = "hosted-websocket"
))]
mod evidence;
