//! Rastok compiler and runtime — the M0 slice: one `.rkg` unit in,
//! a checked plan, a journaled run against real Drivers, a lab of
//! scripted worlds, and a canonical trace out.
//!
//! The build sequence is documented in
//! `docs/architecture/build-order.md`; the spec under `docs/spec/` is
//! normative.

// The kernel's modules re-export here so every existing path —
// `rastok::envelope`, `rastok::runtime::vm` — keeps resolving; the
// crate boundary is real, the surface is unchanged.
pub use rastok_core::{
    boundary, canonical_cbor, connector, data_policy, datatype, envelope, expr, identity, journal,
    loop_proof, node, plan, runtime, schema_annotation, value,
};

pub mod compiler;
pub mod conformance;
pub mod connectors;
pub mod contract;
pub mod diagnostics;
pub mod golden;
pub mod lab;
pub mod openapi;
pub mod render;
pub mod schema_projection;
pub mod stdlib;
