# Specification-to-implementation map

> Planning aid — non-normative. **Updated 2026-08-13 through M1.2 and
> its compiler interludes.** The table first preserves the M0 evidence;
> the M1 extension below records which normative owners gained new
> executable responsibility. Where implementation surfaced a semantic
> ambiguity, it stopped at the boundary as required: the errata live in
> [`../spec/errata.md`](../spec/errata.md), the settled choices in
> [`../spec/decisions.md`](../spec/decisions.md), and the M0 deviations
> in [`m0-exit-notes.md`](m0-exit-notes.md).

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

## M1 extensions now in force

| Landed slice | Normative responsibility | Code owner | Evidence boundary |
|---|---|---|---|
| M1.1 multiport | Named boundary ingress, resolved node/boundary port tables, optional pinned types, and one conformance judgment at every emission | `compiler::{check,lower}`, `plan::{PortDecl,ValidatedExecutionPlan}`, `runtime`, `boundary` | checker negatives, package round trips, runtime pin refusals, gauge identity witness |
| Metadata/literals/IDL reform | Exactly three metadata zones; inferred `app`, sheet-declared `ext`; brace punctuation; text/base64/JSON/assets; structural type aliases | `compiler::{lexer,parser,check,lower}`, `envelope`, `json`, `plan` | schema/path diagnostics, literal boundaries, exact-decimal and asset tests, package/golden pins |
| M1.2 `jsonl` | One canonical JSON line on request, `array<variant>` replies, exact decoding failures, effective output pin | `boundary::{prepare_jsonl,effective_pin}`, `json`, `connectors::Process`, checker and plan validator | real-child round trips, malformed-line refusals, golden request digest |
| Contract interlude | One compiler-layer node contract authority: attribute domains, `Required`/`Optional`/`Defaulted`, profiles, pre-hash default injection; adapter binding before VM/journal | `contract`, `stdlib`, `compiler::check`, `connectors::Process::try_new` | sheet validation, identity equivalence, forged/stripped plan refusals, adapter conformance |
| Parser interlude | Forward lexeme stream with lexer-owned cursor/lookahead; reserved `json`/`nothing`; no implicit yield; semantic region delegation; bounded structural recovery | `compiler::{lexer,parser,syntax}`, `trivia`, `cel`, `json` | success/rejection/diagnostic-location matrices, sibling-preservation regressions, unchanged valid-program hash |
| Plan package v6 | The VM accepts only `ValidatedExecutionPlan`; shared bounded canonical CBOR; v5 body plus decimal call id 4 under a v6 envelope | `plan::{serialization,validation}`, `canonical_cbor`, `identity::encoding` | canonical round trip, typed corruptions, `no_std` decode and emulated Cortex-M33 spike |

The next row to add is M1.3, the author-written typed-error half
(`err = Kind { … }` and mock failures). Until then, connector/machine
failures execute but operator-authored error emissions remain guarded.

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
