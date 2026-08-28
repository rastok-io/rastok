//! The Rastok kernel — everything deterministic, nothing of the OS.
//!
//! This crate holds the vocabulary (`node`), logical types and values,
//! envelopes, the expression chain, the executable plan, the journal's records
//! and trait, the boundary vocabulary and its five traits, and the
//! machine (`runtime`, `runtime::vm`). The world enters exclusively
//! through the boundary traits; Drivers, the compiler, the lab, and
//! every OS concern live in the `rastok` crate above. The frontier is
//! machine-checked: this crate compiles for a bare-metal target in CI.

#![no_std]

extern crate alloc;

// The tests exercise the kernel from a hosted toolchain; the library
// itself never links std.
#[cfg(test)]
extern crate std;

pub mod boundary;
pub mod canonical_cbor;
pub mod connector;
pub mod datatype;
pub mod envelope;
pub mod expr;
pub mod identity;
pub mod journal;
pub mod json;
pub mod loop_proof;
pub mod node;
pub mod plan;
pub mod runtime;
pub mod trivia;
pub mod value;
