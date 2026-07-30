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
| E-DUP-KEY | error | check | [identity.md](identity.md) | a STATIC map/struct literal with two keys that canonicalize equal; a runtime collision is instead a typed language error, not a tool diagnostic |
| E-UNION-OVERLAP | error | check | [identity.md](identity.md) | a `union` with overlapping or identical branches (`int \| int`, `int \| long`, `number \| long`) — no canonical `branch_index` |
| W-DEAD-GRAPH | warning | resolve | [grammar.md](grammar.md) | a top-level graph reachable from no App composition |
| W-DURABILITY-WEAKENED | warning | plan | [delivery-semantics.md](delivery-semantics.md) | a Node or nested Graph explicitly selects a weaker scope than its closest inherited value |
