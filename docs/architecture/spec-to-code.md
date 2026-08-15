# Specification-to-implementation map

> Planning aid — non-normative. **Updated 2026-08-15 through M1.3 B6 and
> the specified M1.3 target.** The table first preserves the M0 evidence;
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
| M1.2 `jsonl` (current v6) | One canonical JSON line on request, legacy `array<variant>` job replies, exact decoding failures, resolved codec-selected ports; M1.3 replaces the wrapper with ordinary `variant` frame emissions | `boundary::{prepare_jsonl,legacy_v6_process_port_type}`, `contract::PortContract`, `json`, `connectors::Process`, checker and plan validator | real-child round trips, malformed-line refusals, golden request digest |
| Contract interlude | One compiler-layer node contract authority: attribute domains, `Required`/`Optional`/`Defaulted`, profiles, pre-hash default injection; adapter binding before VM/journal | `contract`, `stdlib`, `compiler::check`, `connectors::Process::try_new` | sheet validation, identity equivalence, forged/stripped plan refusals, adapter conformance |
| Parser interlude | Forward lexeme stream with lexer-owned cursor/lookahead; structural `nothing`; no implicit yield; semantic region delegation; bounded structural recovery | `compiler::{lexer,parser,syntax}`, `trivia`, `expr`, `json` | success/rejection/diagnostic-location matrices, sibling-preservation regressions, unchanged valid-program hash |
| Plan package v6 | The VM accepts only `ValidatedExecutionPlan`; shared bounded canonical CBOR; v5 body plus decimal call id 4 under a v6 envelope | `plan::{serialization,validation}`, `canonical_cbor`, `identity::encoding` | canonical round trip, typed corruptions, `no_std` decode and emulated Cortex-M33 spike |
| M1.3 B1 core types/values | One materialized value authority and one recursive conformance authority cover the strict JSON Schema, Avro and Arrow families; resolved types own widths/units/fixed bounds instead of being copied into envelopes; decimal128 bounds are type-domain policy, not JSON policy | `value::{Value,Value::validate,ValueError}`, `datatype::{DataType::violation,ConformanceViolation}`, `identity::encoding`, consumed by plan/runtime/VM doors | exact float-width, temporal/interval, UUID, fixed-collection and canonical-key map matrices; nested-path refusals; v6 categorical guard; unchanged existing bytes |
| M1.3 B2 effective ports | Every used or declared boundary/node port has one effective `DataType` and `Declared`/`Inferred` origin in `ValidatedExecutionPlan`; no unsolved variable becomes `variant`; assignability is directional and shared by checker, raw-plan gate, ingress and emissions | `plan::type_resolution`, `ValidatedExecutionPlan::resolved_ports`, `datatype::DataType::is_assignable_to`, compiler diagnostic bridge, runtime pin gate | inferred field-path types, unresolved/conflict negatives, explicit-variant direction matrix, v6 decode re-resolution, Lab/partner/ingress parity |
| M1.3 B3 expressions | The `expr` engine consumes core `DataType`/`Value`; evaluator-only witnesses cannot escape; `is`/`as` share one typed narrowing judge with deterministic structural/tagged union selection and conservative flow facts; v6 refuses the new AST until B8 | `expr::{Signature,Tree,NarrowTarget,ErrorKind}`, `datatype::{Discriminator,NarrowingFailure}`, compiler signature/guard bridge, plan v6 categorical gate | independent CEL-compatible oracle corpus, no-match/ambiguity/discriminator tests, handler-flow check, forged-v6 refusal, no_std |
| M1.3 B4 tagged frontends | Byte-adjacent `IDENT!` alone selects one complete frontend value; the compiler Registry supplies framing and reader policy; JSON accepts every scalar/composite value; `asset` remains a build-bound ordinary expression call | `contract::{FrontendContract,FrontendReader}`, `stdlib::Registry`, `compiler::{lexer,parser,check,syntax}`, `expr`, `json` | custom registered frontend, scalar/exact JSON, adjacency and `!=` maximal-munch matrices, recovery boundaries, unchanged v6 package/hash witness |
| M1.3 B5 uniform port contracts | One resolved Connector port table carries direction, logical type and unwired policy; Process owns `in/out/err/exit`, `codec = bytes`, independent defaulted `err_codec`, and the fixed migration target; no travelling error-kind list or VM port-name routing remains | `contract::{ConnectorContract,PortContract}`, `stdlib::PROCESS`, `compiler::{check,lower}`, `plan::{validation,serialization}`, `runtime`, `boundary` | sheet-defect matrix, fixed/same-as/profile-selected resolution, Registry-free forged-plan refusals, root-sink policy tests, deliberate hash/package pins |
| M1.3 B6 tagged sums | `Case { ... }` constructs any context-compatible tagged union; structural-union destinations receive one explicit HIR wrapping; distinct unions require source `as`; `err` has no special syntax or HIR statement | `compiler::{parser,syntax,check,lower}`, `expr::{Expr::Struct,Expr::Tagged,Expr::As}`, `datatype::{TaggedUnionType,DataType::narrow}`, v6 plan categorical gate | exact/missing case fields, nested optional wrapping, implicit/explicit union conversion pair, ordinary `err` emission, forged constructed tree, unchanged old-program package/hash witness |

## M1.3 target (landing in ordered gates)

| Slice | Normative responsibility | Intended owner boundary | Required evidence |
|---|---|---|---|
| M1.3 logical payload types and uniform ports | One canonical `DataType`/`Value`; complete effective port types; JSON/Avro/Arrow projections including unions; Rastok expressions as a consumer; `IDENT!` frontends; ordinary multi-output completion with Process `err` diagnostics and one typed `exit` per invocation | core `datatype` (type authority landed), then `value`/`expr`/envelope/plan vocabulary plus std compiler, contract, schema-projection and boundary adapters | the ordered gates in [`m1.3-type-system-backlog.md`](m1.3-type-system-backlog.md), one categorical v7 move, external-schema mapping matrices, differential CEL-compatible tests, package sabotage, Lab/real Process parity, no_std and spike |

Core `datatype::DataType` and `value::Value` now own the executable type and
materialized-value subsets without moving the v6 wire ids or semantic hashes.
Their validation and recursive conformance judges are likewise core-owned and
shared by plan constants, metadata, state, ingress and emitted values; JSON
keeps only origin-specific reading/writing rules.
The expression-only `expr::EvalValue` may additionally carry the lazy payload
witness, but cannot cross into plans, envelopes, or journals. Effective port
types now resolve completely in the validated v6 plan and are re-derived on
decode; B8 changes their transport, not the execution invariant. The travelling
`error_set` and compiler-side Process pin bridge are gone in B5. `TypedError`
and the success/failure-only boundary reply are now the isolated pre-B7 seam;
the active v6 `err/exit: variant` rows state that bridge honestly. They are not
the target model and must not acquire new normative responsibilities.

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
