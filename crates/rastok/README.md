# Rastok

<!-- Public narrative: rastok-strategy/NARRATIF.md, "Version anglaise de référence",
     b650bc0. Keep this copy verbatim; propose wording changes in the source. -->

> Rastok is a simulation-first integration platform. A flow and its
> failure scenarios are written together, in a dedicated, typed language
> versioned in Git. The same program runs against a simulated world,
> virtual time and faults included, or against the real systems: you build
> and prove an integration before its partner systems are available.
>
> Every run leaves a replayable journal. An incident becomes a scenario
> that protects the next changes, and a record an auditor can read. The
> language is constrained enough for code written or driven by AI to be
> checked by the compiler and the simulation before it reaches production.

The narrative describes the product vision. The implemented scope and limits
of this experimental alpha are listed below.

A typed DSL and runtime for stateful, effectful integration graphs. The same
validated program runs against real Drivers or scripted Lab outcomes, with
explicit causal records and deterministic simulation.

The CLI provides `check`, `simulate`, `run`, diagnostic `replay`, `new`, and
the read-only `connectors [--json]` capability catalogue.
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

Public destinations for the planned 5 November 2026 opening:
[website](https://www.rastok.com),
[documentation](https://docs.rastok.com),
[repository](https://github.com/rastok-io/rastok),
[binary releases](https://github.com/rastok-io/rastok/releases).
These are release destinations, not a claim that publication has occurred.

License: Apache-2.0.
