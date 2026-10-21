//! Rastok compiler and runtime — the M0 slice: one `.rkg` unit in,
//! a checked plan, a journaled run against real Drivers, a lab of
//! scripted worlds, and a canonical trace out.
//!
//! The build sequence is documented in
//! `docs/architecture/build-order.md`; the spec under `docs/spec/` is
//! normative.

// The hosted crate uses the core's domain types directly. Its Connector
// module adds hosted definitions and Drivers without duplicating core types.
pub use rastok_core::{
    accounting, canonical, canonical_cbor, data_policy, datatype, deployment, envelope, expr,
    instance, journal, loop_proof, node, observation, plan, runtime, schema_annotation, value,
};

pub mod compiler;
pub mod connector;
pub mod contract;
pub mod deployment_config;
pub mod diagnostics;
pub mod golden;
pub mod lab;
pub mod observability;
pub mod openapi;
pub mod recording;
pub mod render;
pub mod schema_projection;
pub mod stdlib;
#[cfg(feature = "hosted-ui")]
pub mod ui;
pub mod worker;
