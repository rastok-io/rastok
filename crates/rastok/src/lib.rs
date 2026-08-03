//! Rastok compiler and runtime.
//!
//! The first executable slice will define this crate's public surface.
//! The build sequence is documented in `docs/architecture/build-order.md`.

pub mod boundary;
pub mod cel;
pub mod compiler;
pub mod diagnostics;
pub mod envelope;
pub mod journal;
pub mod node;
pub mod plan;
pub mod runtime;
pub mod stdlib;
