# Rastok

A typed DSL and runtime for stateful, effectful integration graphs. The same
validated program runs against real Drivers or scripted Lab outcomes, with
explicit causal records and deterministic simulation.

The CLI provides `check`, `simulate`, `run`, diagnostic `replay`, and `new`.
Build with `hosted-ui,hosted-websocket` for the simulation debugger and hosted
HTTP/1.1–2/WebSocket Drivers. HTTP/3 is an independent, optional capability.

This alpha has an in-memory deployment journal. Diagnostic recordings are
neither durable recovery nor share-safe exports. No production-readiness
claim, Kafka, SQL, or Saga implementation is made. The language and library
API are pre-release contracts.

Rust 1.97.1 is the qualified toolchain. Source-checkout documentation,
examples, gallery traces and the complete conformance suite are maintained
together; the crate archive supplies the executable/library sources and
embedded debugger assets, not the full documentation repository.

License: Apache-2.0.
