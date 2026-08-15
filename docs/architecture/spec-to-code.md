# Specification-to-implementation map

> Planning aid — non-normative. **Updated 2026-08-15 through the closed M1.3
> B9 foundation.** The table first preserves the M0 evidence;
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
| M1.2 `jsonl` (historical landing) | One canonical JSON line on request, exact decoding failures and codec-selected ports; its original `array<variant>` reply wrapper is superseded by B7 unit `variant` frames | `boundary::prepare_jsonl`, `contract::PortContract`, `json`, `connectors::Process`, checker and plan validator | real-child round trips, malformed-line refusals, golden request digest; B7 framing matrix |
| Contract interlude | One compiler-layer node contract authority: attribute domains, `Required`/`Optional`/`Defaulted`, profiles, pre-hash default injection; adapter binding before VM/journal | `contract`, `stdlib`, `compiler::check`, `connectors::Process::try_new` | sheet validation, identity equivalence, forged/stripped plan refusals, adapter conformance |
| Parser interlude | Forward lexeme stream with lexer-owned cursor/lookahead; structural `nothing`; no implicit yield; semantic region delegation; bounded structural recovery | `compiler::{lexer,parser,syntax}`, `trivia`, `expr`, `json` | success/rejection/diagnostic-location matrices, sibling-preservation regressions, unchanged valid-program hash |
| Plan package v7 | The VM accepts only `ValidatedExecutionPlan`; shared bounded canonical CBOR; complete canonical type/value/expression registry plus a mandatory total resolved-port table which decode re-derives and compares | `plan::{serialization,validation,type_resolution}`, `canonical_cbor`, `identity::encoding` | canonical round trip, every type/expression/value family, table/type sabotage, `no_std` decode and emulated Cortex-M33 spike |
| M1.3 B1 core types/values | One materialized value authority and one recursive conformance authority cover the strict JSON Schema, Avro and Arrow families; resolved types own widths/units/fixed bounds instead of being copied into envelopes; decimal128 bounds are type-domain policy, not JSON policy | `value::{Value,Value::validate,ValueError}`, `datatype::{DataType::violation,ConformanceViolation}`, `identity::encoding`, consumed by plan/runtime/VM doors | exact float-width, temporal/interval, UUID, fixed-collection and canonical-key map matrices; nested-path refusals; v7 value-family round trip |
| M1.3 B2 effective ports | Every used or declared boundary/node port has one effective `DataType` and `Declared`/`Inferred` origin in `ValidatedExecutionPlan`; no unsolved variable becomes `variant`; assignability is directional and shared by checker, raw-plan gate, ingress and emissions | `plan::type_resolution`, `ValidatedExecutionPlan::resolved_ports`, `datatype::DataType::is_assignable_to`, compiler diagnostic bridge, runtime pin gate | inferred field-path types, unresolved/conflict negatives, explicit-variant direction matrix, v7 transported-table comparison, Lab/partner/ingress parity |
| M1.3 B3 expressions | The `expr` engine consumes core `DataType`/`Value`; evaluator-only witnesses cannot escape; `is`/`as` share one typed narrowing judge with deterministic structural/tagged union selection and conservative flow facts | `expr::{Signature,Tree,NarrowTarget,ErrorKind}`, `datatype::{Discriminator,NarrowingFailure}`, compiler signature/guard bridge, plan v7 expression registry | independent CEL-compatible oracle corpus, no-match/ambiguity/discriminator tests, handler-flow check, v7 round trip, no_std |
| M1.3 B4 tagged frontends | Byte-adjacent `IDENT!` alone selects one complete frontend value; the compiler Registry supplies framing and reader policy; JSON accepts every scalar/composite value; `asset` remains a build-bound ordinary expression call | `contract::{FrontendContract,FrontendReader}`, `stdlib::Registry`, `compiler::{lexer,parser,check,syntax}`, `expr`, `json` | custom registered frontend, scalar/exact JSON, adjacency and `!=` maximal-munch matrices, recovery boundaries |
| M1.3 B5 uniform port contracts | One resolved Connector port table carries direction, logical type and unwired policy; Process owns `in/out/err/exit`, `codec = bytes`, independent defaulted `err_codec`, and the fixed migration target; no travelling error-kind list or VM port-name routing remains | `contract::{ConnectorContract,PortContract}`, `stdlib::PROCESS`, `compiler::{check,lower}`, `plan::{validation,serialization}`, `runtime`, `boundary` | sheet-defect matrix, fixed/same-as/profile-selected resolution, Registry-free forged-plan refusals, root-sink policy tests, deliberate hash/package pins |
| M1.3 B6 tagged sums | `Case { ... }` constructs any context-compatible tagged union; structural-union destinations receive one explicit HIR wrapping; distinct unions require source `as`; `err` has no special syntax or HIR statement | `compiler::{parser,syntax,check,lower}`, `expr::{Expr::Struct,Expr::Tagged,Expr::As}`, `datatype::{TaggedUnionType,DataType::narrow}`, v7 plan gate | exact/missing case fields, nested optional wrapping, implicit/explicit union conversion pair, ordinary `err` emission, v7 constructed-tree round trip |
| M1.3 B7 boundary completion | One correlated Partner completion carries ordered ordinary port drafts and one final typed `exit`; the VM alone mints envelopes and commits the completion transaction. `Process(job)` frames stdout/stderr independently and reports every observed status as `ProcessExit`. | `boundary::{Completion,PortOutput,CompletionGate,ProcessExit}`, `journal::Closure`, `runtime::{TransactionExecutor,vm}`, `connectors::Process`, `lab::MockPartner`, `golden` | malformed/multiple/non-final exit refusals, real and mocked multi-output jobs, lines/jsonl/bytes framing, stderr/non-zero independence, timeout/spawn/decode/capture/overflow exits, correlation/conformance battery, updated causal goldens |
| M1.3 B8 package registry | One categorical v7 publication carries complete canonical types, every allocated materialized value, resolved narrowing/constructor trees and the total effective port table; v6 refuses and no external layout code becomes identity | `plan::{serialization,validation,type_resolution}`, `datatype`, `identity::encoding`, compiler hashing | discriminant registry pins, malformed-family and lying-table sabotage, semantic round trips, deliberate hashes/goldens, no_std builds on Cortex-M33, Cortex-M3 and RV32IMAC, plus the emulated Cortex-M33 spike |
| M1.3 B9 closure | Strict JSON Schema, Avro and Arrow profiles project/reverse exactly or refuse by path and reason; physical union selectors stay outside logical identity; `variant` structure requires narrowing; retired v6 subset/refusal scaffolding is absent | `schema_projection::{json_schema,avro,arrow}`, `expr`, `plan::validation`, compiler fixtures | projection matrices and selector preservation, exact `UnboundAssetCall`, narrowing negatives, cross-layer Process/Lab/JSONL/opaque-data corpus, all portability gates |

## M1.3 foundation (landed in ordered gates)

| Slice | Normative responsibility | Intended owner boundary | Required evidence |
|---|---|---|---|
| M1.3 logical payload types and uniform ports | One canonical `DataType`/`Value`; complete effective port types; JSON/Avro/Arrow projections including unions; Rastok expressions as a consumer; `IDENT!` frontends; ordinary multi-output completion with Process `err` diagnostics and one typed `exit` per invocation | core `datatype`, `value`, `expr`, envelope/plan vocabulary plus std compiler, contract, schema-projection and boundary adapters | the closed ordered gates in [`m1.3-type-system-backlog.md`](m1.3-type-system-backlog.md), one categorical v7 move, external-schema mapping matrices, differential CEL-compatible tests, package sabotage, Lab/real Process parity, no_std and spike |

Core `datatype::DataType` and `value::Value` now own the executable type and
materialized-value domain under the complete v7 registry.
Their validation and recursive conformance judges are likewise core-owned and
shared by plan constants, metadata, state, ingress and emitted values; JSON
keeps only origin-specific reading/writing rules.
The expression-only `expr::EvalValue` may additionally carry the lazy payload
witness, but cannot cross into plans, envelopes, or journals. Effective port
types now resolve completely in the validated v7 plan, travel in the package,
and are independently re-derived on decode. The travelling
`error_set` and compiler-side Process pin bridge are gone in B5. B7 also
removes `TypedError` and the success/failure-only boundary reply. B8 removed
the last representational bridges: raw packages carry exact `ProcessExit` and
`decimal()` carries `number`.
B9 adds the strict external adapters, removes the last v6 subset/refusal
scaffolding, closes unchecked structural access through `variant`, and removes
the hidden Operator runtime-fault producer: explicit `err` is an ordinary
domain output and VM faults commit nothing.

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
