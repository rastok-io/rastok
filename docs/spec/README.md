# The Rastok specification

> **Status:** normative corpus, maintained in place. The text is
> edited directly; git history is the revision record. Tagged
> editions (`v0.0-spec-freeze`, …) mark the versions implementations
> and external readers may pin. An amendment that resolves an entry
> of [`errata.md`](errata.md) marks it resolved in the same change;
> settled choices and their rejected alternatives are recorded in
> [`decisions.md`](decisions.md) so they are not re-litigated without
> new evidence.

This directory defines the language and runtime contracts that an
implementation of Rastok must honor. The documents are organized by
semantic ownership rather than by release layer.

[`annotations.md`](annotations.md) records the executable `@struct` model,
protected standard bindings and generic two-field `@measure(id, unit)`.
Policy, privacy and retained measurement meanings live in
[`observability.md`](observability.md). The [format registry](identity.md) names
the current package and recording formats. The
[Node realignment inventory](../architecture/node-realignment-identity.md)
records the package-only update and unchanged semantic and causal witnesses.

## Documentation rules

1. **One home per decision.** A normative rule lives in exactly one
   specification document; every other document cites it. If two
   documents disagree, the designated home wins and the other document
   is a bug.
2. **The live corpus is the source of truth.** Open questions and
   intentionally deferred behavior are stated where they apply. Private
   design history does not compete with the public contract.
3. **Simulation is executable specification.** Spec-critical behavior
   should have executable evidence: a checked example from the
   [catalogue](../examples/README.md), a golden trace, or a conformance test.
   The seven historical paper derivations are not a substitute for those tests.
4. **Specification and implementation stay explicitly aligned.** Normally an
   observable change updates its normative home and executable evidence in one
   change. A large rework may freeze the specification first only when a
   decision names the ordered code gates, the documents label current versus
   target behavior, and non-executable syntax continues to fail closed. The
   v16 R0–R2/R3–R9 split is such a staged decision.
5. **The normative text stands on its own.** Historical design and qualification reports are explicitly dated and linked;
   they never override current rules. Private planning material is not an
   alternative semantic authority.

## Reader path

`README.md` → `WHITEPAPER.md` (the conceptual model) → this file →
[types.md](types.md) (logical payload types) →
[schema-projections.md](schema-projections.md) (JSON Schema / Avro / Arrow /
OpenAPI profiles and project resolution) →
[observability.md](observability.md) (data annotations, secrets, observations
and exact cost) →
[expressions.md](expressions.md) (pure expressions + tagged frontends) →
[grammar.md](grammar.md) (host surface and delimitation) →
[envelope.md](envelope.md) (identity, metadata, storage) →
[instances.md](instances.md) (Node-owned keys, state scope, lifetime,
logical addresses and the shard boundary) →
[execution-model.md](execution-model.md) (transactions, mailboxes, time) →
[delivery-semantics.md](delivery-semantics.md) (journal, recovery,
boundaries) → [simulation-semantics.md](simulation-semantics.md) (the
Lab: scenarios, mocks, golden traces, experiments, and the reserved
theorem surface) → [identity.md](identity.md) (semantic hash,
`graph_version`, canonical record form) → [stdlib.md](stdlib.md) (the
six Connectors and guest languages) →
[extensions.md](extensions.md) / [loop.md](loop.md) / [retry.md](retry.md) /
[effects-and-sagas.md](effects-and-sagas.md) (Extensions, finite recurrence,
safe repetition, and future compensation) →
[example catalogue](../examples/README.md) (runnable graphs and separately
labelled historical derivations).

## The map

| Document | Normative home of |
|---|---|
| [types.md](types.md) | logical payload/value tree, inference boundary, parametric port types, schema adapters, classification boundary |
| [schema-projections.md](schema-projections.md) | lossless supported JSON Schema, Avro and Arrow mappings plus path-specific refusal vocabulary |
| [observability.md](observability.md) | schema annotations, secret handling, individual measurement retention and policy-aware observation/export; reports deferred |
| [expressions.md](expressions.md) | Rastok expression contract, CEL-compatible core, `IDENT!` tagged frontends, JSON values, narrowing |
| [grammar.md](grammar.md) | host syntax, keywords, LL(1) productions, expression/frontend positions and delimitation, simulation grammar |
| [envelope.md](envelope.md) | envelope structure, `meta.sys`, identity framing, storage and wire form, provenance |
| [instances.md](instances.md) | Node-owned instance keys, canonical logical addresses, Operator-local mutable state, lifetime, lifecycle containment, placement/shard separation |
| [execution-model.md](execution-model.md) | Executor and manager transactions, mailboxes M:N, Dispatch, ordering, dated events, timeouts, windows, `advance` |
| [delivery-semantics.md](delivery-semantics.md) | journal records, cursors, backpressure, boundary WAL, recovery, sagas × lifecycle |
| [simulation-semantics.md](simulation-semantics.md) | simulations, scenarios, fixtures, mocks, expectations, golden traces, coverage, production-to-fixture projection |
| [stdlib.md](stdlib.md) | Connector contracts, the six initial Connectors, `behavior`, pure guest programs, observability, MCP |
| [extensions.md](extensions.md) | Node / Graph / Executor / Connector / Extension taxonomy, the Connector-owned `Contract` plus canonical Extension mechanism, admission tests, qualified names |
| [loop.md](loop.md) | finite temporal recursion over Operator, Graph, and outbound Call hosts; manager commands, fuel, delay, recovery, and proofs |
| [retry.md](retry.md) | bounded repetition of one prepared outbound Call operation; terminal safety, held batches, closure-based delay, Closing, and recovery |
| [effects-and-sagas.md](effects-and-sagas.md) | effect classes and the deferred Saga Extension direction |
| [identity.md](identity.md) | semantic hash, `graph_version` allocation and manifest, canonical record form |
| [diagnostics.md](diagnostics.md) | diagnostic-code registry, stability contract, lazy assignment |

Known internal contradictions awaiting a spec-side fix are tracked in
[errata.md](errata.md).
