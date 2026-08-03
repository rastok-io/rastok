//! Rastok compiler and runtime — the M0 slice: one `.rkg` unit in,
//! a checked plan, a journaled run against real adapters, a lab of
//! scripted worlds, and a canonical trace out.
//!
//! The build sequence is documented in
//! `docs/architecture/build-order.md`; the spec under `docs/spec/` is
//! normative.

pub mod boundary;
pub mod cel;
pub mod compiler;
pub mod connectors;
pub mod diagnostics;
pub mod envelope;
pub mod golden;
pub mod journal;
pub mod lab;
pub mod node;
pub mod plan;
pub mod render;
pub mod runtime;
pub mod stdlib;
