//! Hosted socket machinery shared by the network Connector families.
//!
//! This module is deliberately private: protocol-library and async-runtime
//! types stop here. The synchronous Driver boundary exchanges only Rastok
//! facts with the VM.

// H3 lands the physical seam before H4-H6 attach concrete Drivers. Keeping
// the staged surface private and explicitly allowed avoids turning it into a
// public API merely to silence dead-code lints between ordered gates.
#[allow(dead_code)]
mod pump;

#[cfg(any(feature = "hosted-http12", feature = "hosted-http3"))]
#[allow(dead_code)]
mod tls;

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
