# M0 build order

> Implementation slice — non-normative. **Completed 2026-08-04**: all
> seventeen stages built and exited, followed by seven adversarial
> review passes (reworks recorded in the git history) and the
> two-machine reproduction ([`m0-exit-notes.md`](m0-exit-notes.md)).
> Notable deltas against the plan below: the hand-written diagnostic
> renderer sufficed — the ariadne fallback of stage 16 was never
> needed; the discrete `advance` sketched for stage 12 landed through
> the review cycle rather than the stage itself; and stages 14–15
> produced `lab`/`golden` in the leaner as-built shapes documented in
> [`components.md`](components.md).

This document orders the construction of the modules named in
`06-rastok-modules-overview` so that every stage compiles, carries its
own tests, and extends a surface that can already be exercised. It is a
dependency-driven sequence, not a schedule; dates and effort live
outside this repository.

Ground rules, inherited from [`m0-slice.md`](m0-slice.md) and
[`spec-to-code.md`](spec-to-code.md):

- base vocabulary is built before its consumers; no stage depends on a
  later stage;
- a stage exits with the smallest focused tests that prove its rule,
  drawn from the minimal-evidence list of the M0 slice;
- syntax that can reach a deferred feature receives its explicit
  diagnostic at the first stage able to parse it;
- when a stage reveals a semantic ambiguity, work stops at the boundary
  and the question is resolved in [`../spec/`](../spec/) first.

## Stages

### Stage 0 — workspace warm-up

Confirm the toolchain, lints, and test harness from a clean checkout.
No behavior yet.

Exit: `cargo test`, `cargo clippy`, and `cargo fmt --check` pass on a
fresh clone.

### Stage 1 — `node`

The shared graph vocabulary: `Path`, `Kind`, `Port`, `EffectClass`,
`Effect`, `Behavior`, `Trigger`.

Exit: unit tests for construction, equality, and ordering of paths and
ports.

### Stage 2 — `diagnostics`

`Span`, `Severity`, `Phase`, `Diagnostic`, `Bag`, and a stable
rendering.

Exit: one rendered diagnostic asserted byte-for-byte, per
[`../spec/diagnostics.md`](../spec/diagnostics.md).

### Stage 3 — `datatype`, `value`, and `envelope`

The core `DataType` and materialized `Value` authorities; then `Encoding`,
`Data`, `Id`, `MetaSys`, `Meta`, `Envelope`, and the canonical identity
derivation. `envelope::Value` is only a re-export of core `value::Value`.

Exit: round-trip and invalid-shape tests, plus repeatability of `Id`
across equivalent construction orders, per
[`../spec/envelope.md`](../spec/envelope.md) and
[`../spec/identity.md`](../spec/identity.md).

### Stage 4 — `compiler` front end

Lexer and parser for the selected M0 grammar subset, producing
`SyntaxTree` with source spans.

Exit: accepted and rejected `.rkg` fixtures with spans, per
[`../spec/grammar.md`](../spec/grammar.md).

### Stage 5 — Rastok expressions (`expr`)

The isolated expression consumer: compilation (`Signature`, `Tree`, native
diagnostics) and evaluation (`EvalValue`, `Bindings`, `Error`) over the core
`DataType` and `Value`. `EvalValue` may carry the lazy payload witness during
one evaluation but never enters a plan, envelope, or journal. Independent of
Stage 4 and may proceed in parallel with it.

Exit: conformance fixtures for the admitted CEL-compatible subset and named
Rastok divergences, including error cases.

### Stage 6 — `compiler` middle

`Unit` merge, resolution, and checking into `Hir` and `NodeDecl`.
Guarded refusals become real here: jq and SQL guests, catalogue
connectors without Drivers, and unsupported aspect or Loop syntax all
produce explicit diagnostics.

Exit: at least one stable diagnostic that covers a real semantic rule
rather than a parser accident.

### Stage 7 — `stdlib`

`ConnectorContract` for `Process` and the compiler-layer
`FrontendContract` registry, wired into parser and checker.

Exit: a fixture rejected for violating the `Process` contract sheet.

### Stage 8 — `plan`

Lowering into `ExecutionPlan`, `Handler`, `Slot`, `SymbolTable`;
`SemanticHash` over the canonical HIR; the `Compilation` result.

Exit: identical hashes across equivalent construction orders, and
distinct hashes across semantically distinct programs.

### Stage 9 — `journal`

`Record` and its variants, the identity recipes in `ids`, the
`Journal` trait, `CloseVerdict`, and `MemJournal`.

Exit: ordering and close-once invariant tests, per
[`../spec/delivery-semantics.md`](../spec/delivery-semantics.md).

### Stage 10 — `boundary`

The world-facing traits (`InboundConnectorDriver`,
`OutboundConnectorDriver`, `Supervisor`) and the VM's explicit logical-time
input
and the journal-driven machinery: `IngressSubmission`,
`IngressEnvelopeProvenance`, `Intent`, `Response`, `CallId`, `Dispatcher`,
`DatedPayload`, `LifecycleEvent`.

Exit: dispatcher tests against a scripted Outbound Driver, asserting journaled
intents and correlated responses.

### Stage 11 — `runtime` core

One atomic node transaction: `InstanceState`, `Mailboxes`,
`EmissionBuffer`, `HandlerExecutor`.

Exit: deterministic single-transaction tests, including emission order
and state isolation between keys.

### Stage 12 — `runtime` loop

`Scheduler` and `Vm` over plan, journal, and boundary; discrete
advance; run-to-completion semantics per
[`../spec/execution-model.md`](../spec/execution-model.md).

Exit: end-to-end in-memory execution of the M0 fixture graph.

### Stage 13 — `connectors`

The real `Process` Driver over a harmless deterministic child
process, behind the same intent and outcome types as the Lab path.

Exit: the App integration test named in the M0 minimal evidence.

### Stage 14 — `lab`

Scenario parsing (`Scenario`, `Expectation`, `Quantifier`, `Fixture`)
and controlled execution (family mock Drivers, scenario origin/`advance`,
`ScenarioRunner`) over the same `Vm`.

Exit: a repeatable Lab scenario for the same graph, with a scripted
`Process` outcome, per
[`../spec/simulation-semantics.md`](../spec/simulation-semantics.md).

### Stage 15 — golden evidence

`GoldenProjection` and `Diff`: the canonical projection of causal
records and the first meaningful divergence.

Exit: one golden trace and one intentional mismatch whose diff
identifies the semantic difference.

### Stage 16 — `cli`

Binary assembly: `check`, `run`, `simulate`.

Exit: the complete minimal-evidence list of
[`m0-slice.md`](m0-slice.md) passes from a clean checkout; the result
reproduces on a second machine.

## Reading the order

Stages 1–3 are pure data and carry no control flow. Stages 4–8 build
the compilation pipeline; Stage 5 is the only one that may run in
parallel. Stages 9–13 build the execution side, journal-first because
both the boundary and the runtime write through it. Stages 14–16 close
the loop with the Lab and the user-facing binary.

A stage may begin before its predecessor is polished, but not before
the predecessor's exit tests pass: exit tests are the contract the next
stage builds on.
