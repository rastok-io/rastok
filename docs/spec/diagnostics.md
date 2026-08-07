# Diagnostics catalog

> **Status:** normative — under review.
> This file is the single registry of tool-output diagnostic codes.

**Scope.** This catalog norms diagnostics emitted by *tools* (compiler,
planner, sim runner) — they never appear in a journal or a golden trace.
Runtime typed errors (`JoinExpired`, `LoopExhausted`, `SagaRolledBack`,
`OutcomeUnknown`, …) are **language material** : err-port payloads,
part of err sets, hashed into the semantic identity and compared in the
causal core. They live in their home specs, not here.

## Rules

1. **Named codes** : `E-` (error) / `W-` (warning) + UPPER-KEBAB.
   A published code is **never renamed and never reused**.
2. **Stability contract** : stable forever = the code + its structured
   payload fields (what tests and tools key on). Never stable = the
   message text (pedagogy improves freely). Machine output
   (`rastok build --json`) is `{code, severity, spans, fields}` ;
   text is rendering.
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
| E-UNION-OVERLAP | error | check | [identity.md](identity.md) | a `union` with overlapping or identical branches (`int \| int`, `int \| long`, `number \| long`) — no canonical `branch_index` |
| E-LEX-CHAR | error | parse | [grammar.md](grammar.md) | a character outside the lexical grammar; field `character` |
| E-LEX-UNTERMINATED-STRING | error | parse | [grammar.md](grammar.md) | a string literal that never closes |
| E-LEX-UNTERMINATED-COMMENT | error | parse | [grammar.md](grammar.md) | a block comment that never closes |
| E-LEX-UNTERMINATED-BLOCK | error | parse | [grammar.md](grammar.md) | a raw guest block whose braces never balance |
| E-PARSE-EXPECTED | error | parse | [grammar.md](grammar.md) | a token breaking the expected production; fields `expected`, `found` |
| E-PARSE-DEFERRED | error | parse | [grammar.md](grammar.md) | syntax admitted by the grammar but outside the subset this build implements; field `construct` |
| E-APP-COUNT | error | resolve | [grammar.md](grammar.md) | a deployed unit without exactly one `: App` graph; field `count` |
| E-DUP-NODE | error | resolve | [grammar.md](grammar.md) | a node name declared twice in one graph; field `name` |
| E-RESOLVE-UNKNOWN | error | resolve | [grammar.md](grammar.md) | a name resolving to no declaration; field `name` |
| E-PORT-UNKNOWN | error | check | [grammar.md](grammar.md) | a port the addressed node does not expose; field `port` |
| E-META-SYS-READONLY | error | check | [envelope.md](envelope.md) | a write reaching the read-only `meta.sys` namespace (§6: there is no write) |
| E-CEL-EXPR | error | check | [grammar.md](grammar.md) | a CEL expression fault, adapted from the CEL chain; field `detail` |
| E-CHECK-DEFERRED | error | check | [grammar.md](grammar.md) | grammar admitted but outside the executable subset of this build; field `construct` |
| E-CONNECTOR-UNAVAILABLE | error | check | [stdlib.md](stdlib.md) | a connector type with no adapter in this build — declared catalogue or unknown; field `type` |
| E-CONTRACT-ATTR | error | check | [stdlib.md](stdlib.md) | an attribute violating its connector contract sheet; fields `attribute`, `detail` |
| E-CONTRACT-REQUIRED | error | check | [stdlib.md](stdlib.md) | a mandatory contract attribute left undeclared; field `attribute` |
| E-DUP-BINDING | error | check | [grammar.md](grammar.md) | a binding name shadowing a visible binding or a reserved handler name; field `name` |
| E-PORT-DIRECTION | error | check | [grammar.md](grammar.md) | a port used against its direction — edge, handler, or emission side; fields `port`, `direction` |
| E-GRAPH-CYCLE | error | check | [grammar.md](grammar.md) | a wiring cycle among nodes — edges form a DAG; field `node` |
| E-PORT-CONFLICT | error | check | [grammar.md](grammar.md) | a port name declared or inferred with conflicting roles, redeclared, or declared on a connector; field `port` |
| E-PARTITION-MISMATCH | error | check | [execution-model.md](execution-model.md) | a join input arriving under a different partition than the join requires — an explicit re-key step is needed; reserved for the joins slice; fields `port`, `expected`, `found` |
| E-DUP-EDGE | error | check | [envelope.md](envelope.md) | the same source endpoint wired twice to the same target, compared after desugaring — the deliveries would derive one call ref; field `edge` |
| E-DUP-LAB-IDENTITY | error | resolve | [simulation-semantics.md](simulation-semantics.md) | a duplicated lab identity — simulation, scenario, or mock of one connector at one level; fragments merge, homonyms refuse; field `name` |
| E-INTERNAL-PLAN | error | plan | [plan-package.md](../architecture/plan-package.md) | the lowering emitted a plan its own validator refuses — a compiler defect surfaced as a diagnostic, never a panic; field `detail` |
| E-PLAN-DEPTH | error | check, plan | [plan-package.md](../architecture/plan-package.md) | a program nesting deeper than the format reader's budget (`MAX_NESTING`) — gated at check on the parsed expression, before any recursive pass, and again at plan on the exact serialized depth; field `depth` (plan phase) |
| W-DEAD-GRAPH | warning | resolve | [grammar.md](grammar.md) | a top-level graph reachable from no App composition |
| W-UNWIRED-OUTPUT | warning | check | [delivery-semantics.md](delivery-semantics.md) | a non-`err` node output wired to no edge and not discarded with `-> nothing`; `err` is exempt — it bubbles to the root error sink; fields `node`, `port` (severity divergence recorded as erratum E-2026-08-03-2) |
| W-DURABILITY-WEAKENED | warning | plan | [delivery-semantics.md](delivery-semantics.md) | a Node or nested Graph explicitly selects a weaker scope than its closest inherited value |
