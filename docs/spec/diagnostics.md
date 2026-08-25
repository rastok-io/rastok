# Diagnostics catalog

> **Status:** normative — under review.
> This file is the single registry of tool-output diagnostic codes.

**Scope.** This catalog norms diagnostics emitted by *tools* (compiler,
planner, sim runner) — they never appear in a journal or a golden trace.
Runtime outcome cases (`JoinExpired`, `LoopExhausted`,
`SagaRolledBack`, `OutcomeUnknown`, …) are **language material**:
ordinary typed port payloads, hashed through their resolved types and
compared in the causal core. They live in their home specs, not here.

The current `E-EXPR` and `IDENT!` rows describe the expression registry first
published in v7; categorical package v11 retains it unchanged. New or
renamed checks are registered only when their focused negative tests
land, never pre-allocated by a planning pass.
`E-EXPR` supersedes the unreleased `E-CEL-EXPR` spelling as part of B3;
no public diagnostic contract ever carried the old code.

## Rules

1. **Named codes** : `E-` (error) / `W-` (warning) + UPPER-KEBAB.
   A published code is **never renamed and never reused**.
2. **Stability contract** : stable forever = the code + its structured
   payload fields (what tests and tools key on). Never stable = the
   message text (pedagogy improves freely). The current CLI renders
   text only; when a machine-output mode lands, its stable shape is
   `{code, severity, spans, fields}` rather than the rendering text.
3. **Registration** : any change that introduces a code MUST add its row
   here in the same commit. This file is the collision authority.
4. **Lazy assignment** : many specification rules still describe
   errors and warnings without a named code. Each receives its code
   **when its check is implemented**, in the same change as its negative
   test. Until then, the prose rule remains normative.

## Registry

| Code | Severity | Phase | Home spec | Meaning (informative) |
|---|---|---|---|---|
| E-LAB-DUP | error | resolve | [grammar.md](grammar.md) | more than one logical lab for the same App |
| E-LAB-NO-APP | error | resolve | [grammar.md](grammar.md) | a lab whose name binds to no `: App` graph in the unit |
| E-SIM-TARGET | error | resolve | [simulation-semantics.md](simulation-semantics.md) | `simulation … on <graph>` targeting a non-App graph (deferred capability) |
| E-EXPERIMENT-PROFILE | error | resolve | [grammar.md](grammar.md) | experiment without exactly one constant `profile` |
| E-SAGA-DURABILITY | error | plan | [effects-and-sagas.md](effects-and-sagas.md) | a Saga whose effective durability resolves to `none` |
| E-COMPENSATE-KEY | error | plan | [effects-and-sagas.md](effects-and-sagas.md) | compensation without a stable business key |
| E-REPLAY-VERSION-MISMATCH | error | replay | [identity.md](identity.md) | journal segment hash ≠ loaded graph hash ; replay refused |
| E-DUP-KEY | error | check | [identity.md](identity.md) | a STATIC map/struct literal with two keys that canonicalize equal; a runtime collision is instead a typed language error, not a tool diagnostic; field `detail` |
| E-UNION-OVERLAP | error | check | [identity.md](identity.md) | a structural union (`A \| B`) with overlapping or identical branches (`int \| int`, `int \| long`, `number \| long`) — no unique branch; a tagged union may deliberately distinguish equal-shaped cases by name; field `detail` |
| E-LEX-CHAR | error | parse | [grammar.md](grammar.md) | a character outside the lexical grammar; field `character` |
| E-LEX-UNTERMINATED-STRING | error | parse | [grammar.md](grammar.md) | a string literal that never closes |
| E-LEX-UNTERMINATED-COMMENT | error | parse | [grammar.md](grammar.md) | a block comment that never closes |
| E-LEX-UNTERMINATED-BLOCK | error | parse | [grammar.md](grammar.md) | a raw guest block whose braces never balance |
| E-PARSE-EXPECTED | error | parse | [grammar.md](grammar.md) | a token breaking the expected production; fields `expected`, `found` |
| E-JSON-VALUE | error | parse | [grammar.md](grammar.md) | a `json!` region the house reader refuses; field `detail` (JSON is not the Rastok expression grammar — its faults carry their own code) |
| E-PARSE-DEFERRED | error | parse | [grammar.md](grammar.md) | syntax admitted by the grammar but outside the subset this build implements; field `construct` |
| E-APP-COUNT | error | resolve | [grammar.md](grammar.md) | a deployed unit without exactly one `: App` graph; field `count` |
| E-DUP-NODE | error | resolve | [grammar.md](grammar.md) | a node name declared twice in one graph; field `name` |
| E-RESOLVE-UNKNOWN | error | resolve | [grammar.md](grammar.md) | a name resolving to no declaration; field `name` |
| E-PORT-UNKNOWN | error | check | [grammar.md](grammar.md) | a port the addressed node does not expose; field `port` |
| E-META-SYS-READONLY | error | check | [envelope.md](envelope.md) | a write reaching the read-only `meta.sys` namespace (§6: there is no write) |
| E-EXPR | error | check | [grammar.md](grammar.md) | a Rastok expression fault, adapted from the isolated expression compiler; field `detail` |
| E-CHECK-DEFERRED | error | check | [grammar.md](grammar.md) | grammar admitted but outside the executable subset of this build; field `construct` |
| E-INSTANCE | error | check | [instances.md](instances.md) | a malformed or repeated `instances`/`lifetime` attribute, a superseded `partition`/`per` spelling, or an instance key without one total canonical type; field `detail`; post-solver type variants additionally carry `name` |
| E-CONNECTOR-UNAVAILABLE | error | check | [stdlib.md](stdlib.md) | a connector type with no Driver in this build — declared catalogue or unknown; field `type` |
| E-CONTRACT-ATTR | error | check | [stdlib.md](stdlib.md) | an attribute violating its connector contract sheet; fields `attribute`, `detail` |
| E-CONTRACT-REQUIRED | error | check | [stdlib.md](stdlib.md) | a mandatory contract attribute left undeclared; field `attribute` |
| E-DUP-BINDING | error | check | [grammar.md](grammar.md) | a binding name shadowing a visible binding or a reserved handler name; field `name` |
| E-PORT-DIRECTION | error | check | [grammar.md](grammar.md) | a port used against its direction — edge, handler, or emission side; fields `port`, `direction` |
| E-GRAPH-CYCLE | error | check | [grammar.md](grammar.md) | a wiring cycle among nodes — edges form a DAG; field `node` |
| E-PORT-CONFLICT | error | check | [grammar.md](grammar.md) | a port name declared or inferred with conflicting roles, redeclared, or declared on a connector; field `port` |
| E-META-ZONE | error | check | [envelope.md](envelope.md) | a metadata path outside the three zones (`sys`/`app`/`ext`), outside a zone's vocabulary, a dynamic `meta[…]` access, or a Lab write into the application's zone (D-2026-08-10-1); field `path` |
| E-META-TYPE | error | check | [envelope.md](envelope.md) | one `meta.app` field written with two incompatible types — both sites carried as spans; field `field` |
| E-META-PREFIX | error | check | [envelope.md](envelope.md) | two flat keys where one prefixes the other (`audit` vs `audit.origin`) — the nested projection cannot hold both; field `field` |
| E-ASSET | error | check | [grammar.md](grammar.md) | an `asset(…)` expression that cannot resolve at build — missing file, App-root escape, per-asset (8 MiB) or total (32 MiB) budget breach, a Lab position, wrong arity, or a non-literal text argument (D-2026-08-10-5); field `path` (empty when the call itself is malformed) |
| E-DUP-TYPE | error | resolve | [grammar.md](grammar.md) | a named type declared twice in the unit (D-2026-08-10-8); field `name` |
| E-TYPE-RECURSIVE | error | check | [grammar.md](grammar.md) | a named type reaching itself through its own expansion — recursion refuses in v1, the faulty reference underlined (D-2026-08-10-8); field `name` |
| E-TYPE-DUP | error | check | [grammar.md](grammar.md) | a duplicate field or symbol inside one type declaration — the duplicate is underlined (D-2026-08-10-8); fields `name`, `field` |
| E-TYPE-INVALID | error | check | [types.md](types.md) | a source type or inferred value violates a canonical `DataType` constructor invariant (invalid map key, decimal parameter, table row, or duplicate member); field `detail` |
| E-TYPE-UNRESOLVED | error | check | [types.md](types.md) | an open source constraint reaches a position that requires one closed effective type (for example `struct { .. }` or an empty collection without contextual element type); field `type` |
| E-CONTENT-TYPE | error | check | [decisions.md](decisions.md) | a content-typed byte form (`base64!(ct)`, `asset(path, ct)`) outside whole-payload position — a nested field has no `meta.ext` of its own (D-2026-08-10-5); field `tag` |
| E-TYPE-BUDGET | error | check | [grammar.md](grammar.md) | a type expansion beyond a structural budget — depth (128), nodes (4096), width (256), or alias links followed (256) — with the MEASURED value in the payload (D-2026-08-10-8); fields `name` + `depth`/`nodes`/`count`/`aliases` |
| E-NAME-RESERVED | error | resolve | [grammar.md](grammar.md) | a reserved word in a binding position: `nothing` or `true`/`false`/`null`; a node named `meta`/`data`/`sys`/`app`/`ext`/`instance`; a binding named `data`/`meta`/`instance`; or a user `type` spelling a builtin type. Frontend names are ordinary identifiers; only their adjacent `!` has structural meaning. One naming policy, refusal at the declaration; field `name` |
| E-BRACE-MIXED | error | check | [grammar.md](grammar.md) | a braced value mixing ordinary data fields with reserved envelope roots (`data`, `meta` — `mime_type` left the list with D-2026-08-10-4) — wrap the data in `data = { … }`; never a silent reinterpretation (D-2026-08-09-1); field `field` |
| E-PORT-TYPE | error | check | [grammar.md](grammar.md) | a typed destination cannot accept or uniquely wrap its value: an unsolved port variable, incompatible edge/emission/ingress/state/case-field requirement, zero or several contextual union branches, a dynamic `variant` without narrowing, or an implicit conversion between distinct union types; fields `port` (destination name), `expected`, `found`. Since M1.3 B2, validation owns a total resolved port table and never substitutes `variant` for an unsolved variable. |
| E-DUP-EDGE | error | check | [envelope.md](envelope.md) | the same source endpoint wired twice to the same target, compared after desugaring — the deliveries would derive one call ref; field `edge` |
| E-DUP-LAB-IDENTITY | error | resolve | [simulation-semantics.md](simulation-semantics.md) | a duplicated lab identity — simulation, scenario, or mock of one connector at one level; fragments merge, homonyms refuse; field `name` |
| E-INTERNAL-PLAN | error | plan | [plan-package.md](../architecture/plan-package.md) | the lowering emitted a plan its own validator refuses — a compiler defect surfaced as a diagnostic, never a panic; field `detail` |
| E-PLAN-DEPTH | error | check, plan | [plan-package.md](../architecture/plan-package.md) | a program nesting deeper than the format reader's budget (`MAX_NESTING`) — gated at check on the parsed expression (the parser also bounds its production recursion at the same budget, one unit per syntactic level), and again at plan on the exact serialized depth; field `depth` in both phases |
| W-DEAD-GRAPH | warning | resolve | [grammar.md](grammar.md) | a top-level graph reachable from no App composition |
| W-UNWIRED-OUTPUT | warning | check | [delivery-semantics.md](delivery-semantics.md) | an emitted `RequireWire` output has no edge. B5 removed the name-based `err` exemption: `RootSink` is exempt by resolved plan policy, while forwarding and discard remain explicit edges. If the warning is accepted, the emission stays journaled/observable but creates no downstream delivery; fields `node`, `port` |
| W-DURABILITY-WEAKENED | warning | plan | [delivery-semantics.md](delivery-semantics.md) | a Node or nested Graph explicitly selects a weaker scope than its closest inherited value |
