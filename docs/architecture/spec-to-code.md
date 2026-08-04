# Specification-to-implementation map

> Planning aid — non-normative. **Status 2026-08-04**: every row's
> initial evidence exists and runs in CI — parse/check fixtures with
> goldens, the diagnostic registry grown change-by-change, the
> end-to-end journal assertions, the golden trace with its named-diff
> mismatch, and the real-vs-scripted Process pair behind one seam.
> Where implementation surfaced a semantic ambiguity, it stopped at
> the boundary as required: nine errata in
> [`../spec/errata.md`](../spec/errata.md) record them; the assumed
> scope deviations live in [`m0-exit-notes.md`](m0-exit-notes.md).

This table prevents implementation work from silently losing a
normative requirement. Responsibilities are logical boundaries for M0;
they are not promised crate names.

| Normative source | M0 implementation responsibility | Initial evidence |
|---|---|---|
| [`README.md`](../spec/README.md) | Preserve the specification's authority and reading order in reviews. | Architecture and code changes link the governing rule. |
| [`grammar.md`](../spec/grammar.md) | Source front end and syntax representation for the selected M0 subset. | Accepted and rejected `.rkg` fixtures with source spans. |
| [`diagnostics.md`](../spec/diagnostics.md) | Typed internal errors and stable diagnostic rendering. | Semantic failure fixture with code, span, and useful message. |
| [`envelope.md`](../spec/envelope.md) | Owned envelope values and canonical boundary representation. | Round-trip and invalid-shape tests for values used by M0. |
| [`identity.md`](../spec/identity.md) | Canonical plan identity, ordering, and hashing inputs. | Repeatability tests across equivalent construction orders. |
| [`execution-model.md`](../spec/execution-model.md) | Single-worker state transition, queues, and scheduling rules required by the slice. | Deterministic step and end-to-end execution tests. |
| [`delivery-semantics.md`](../spec/delivery-semantics.md) | Typed journal records and Process outcome handling without binding the API to a physical layout. | Ordered journal assertions for App and Lab runs. |
| [`simulation-semantics.md`](../spec/simulation-semantics.md) | Lab scenario inputs, controlled handlers, and golden comparison. | Repeatable scenario and intentional mismatch diff. |
| [`stdlib.md`](../spec/stdlib.md) | The Process connector contract used by M0; other catalogue entries remain unimplemented. | One real handler and one controlled handler behind the same intent/outcome types. |
| [`aspects.md`](../spec/aspects.md) | Represent and validate only aspects exercised by the selected program. | Checker fixtures for each aspect admitted into M0. |
| [`effects-and-sagas.md`](../spec/effects-and-sagas.md) | Preserve effect classification and explicit outcomes. Live Saga management is deferred. | Effect boundary tests; no placeholder Saga subsystem. |
| [`loop.md`](../spec/loop.md) | Parse and reject or validate Loop constructs consistently with the chosen subset. Live Loop recovery is deferred. | Explicit supported/unsupported diagnostic; no inert runtime manager. |

## Keeping the map honest

- `Planned` behavior is not described as implemented in public
  documentation.
- A deferred feature gets a clear diagnostic if source syntax can reach
  it; it must not fail later by accident.
- When a spec rule first becomes executable, add the smallest focused
  test that proves it and update this map if its owner changed.
- A new crate is recorded only after a real dependency or ownership
  boundary appears.
- If implementation work reveals a semantic ambiguity, stop at the
  boundary and resolve it in `docs/spec/` before encoding a private
  interpretation in code.
