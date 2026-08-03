# Specification errata

> Known internal contradictions in the normative corpus, awaiting a
> spec-side fix. Rule 1 of [README.md](README.md) applies: when two
> documents disagree, the designated home wins; the entries below name
> the losing text so it is corrected rather than silently reinterpreted.
> An erratum is not a license to improvise — until the fix lands, the
> designated home is the contract.

## E-2026-07-31-1 — grammar.md still targets the retired builder API

- **Where:** [grammar.md](grammar.md), section "What the parser must
  produce".
- **Contradiction:** the section requires a `.rkg` source to compile to
  "the same `Graph` value as the Rust builder API"
  (`Graph::new` / `add` / `from_config`), a relic of a retired
  prototype. The designated homes disagree:
  [identity.md §1](identity.md) defines the checked form as the
  canonical HIR, and `WHITEPAPER.md` §2 names the compilation target
  the `ExecutionPlan`.
- **Resolution direction:** rewrite the section to state that the
  parser feeds the pipeline `AST → HIR → ExecutionPlan`; no Rust
  builder API is part of the contract.
- **Found by:** adversarial architecture review, 2026-07-31.

## E-2026-07-31-2 — `theorem` wording vs its grammar productions

- **Where:** [grammar.md](grammar.md), "`theorem` blocks" vs the formal
  productions (`theorem_decl`, `lab_body`).
- **Contradiction:** the prose says `theorem` "is not part of the
  grammar", while the formal grammar defines `theorem_decl` as a parsed
  reserved block. The intended behavior (parse structurally, refuse
  with an explicit `unsupported` diagnostic) is stated in
  [simulation-semantics.md §11](simulation-semantics.md).
- **Resolution direction:** editorial — reword the prose to "parsed as
  a reserved surface and refused with `unsupported`; carries no
  executable semantics in this corpus".
- **Found by:** adversarial architecture review, 2026-07-31.

## E-2026-08-03-1 — `envelope.md` §5.1 is silent on failed decodes

- **Where:** [envelope.md §5.1](envelope.md), lazy rule 2 ("Access →
  transcode once").
- **Gap:** the rule fixes the cost of a *successful* materialization
  — one transcode, cached — but does not say what a *failed* decode
  does: whether the typed decode error is memoized like a value, or
  whether a later access may retry. The two policies diverge
  observably (a retried decoder may succeed after its codec set
  changes).
- **Interim behavior (implementation):** a failed decode caches
  nothing; the typed error reaches the caller intact and a later
  access may retry. "Transcode once" reads as "at most one successful
  transcode".
- **Resolution direction:** state the failure policy in §5.1. If
  decode is declared pure and deterministic, memoize the failure with
  a domain `DecodeError`; if retry is intended, say so explicitly.
- **Found by:** external code review (second pass), 2026-08-03.

## E-2026-08-03-2 — unwired data outputs: warning or error?

- **Where:** [grammar.md](grammar.md) (wiring prose) vs
  [delivery-semantics.md](delivery-semantics.md) (routing rules).
- **Contradiction:** grammar.md calls an unwired non-`err` data
  output a *warning*; delivery-semantics.md treats an unrouted data
  emission as an *error*. Both agree `-> nothing` is the explicit
  discard and that `err` bubbles to the root sink.
- **Interim behavior (implementation):** `W-UNWIRED-OUTPUT`, a
  warning — the plan still ships. The runtime never silently drops:
  unwired `err` emissions surface at the root error sink, and
  unwired data emissions are visible at check time.
- **Resolution direction:** pick one severity and align both specs.
  The warning reading keeps exploratory graphs compiling; the error
  reading serves the "nothing silent" axiom better once `-> nothing`
  is cheap to write everywhere.
- **Found by:** external code review (fourth pass), 2026-08-03.

## E-2026-08-03-3 — the idempotency-key rule vs the Process sheet

- **Where:** [effects-and-sagas.md](effects-and-sagas.md) (the
  general rule: configurable connectors carrying `irreversible`
  require an idempotency key) vs [stdlib.md](stdlib.md) (the Process
  sheet, which lists no `idempotency_key` attribute).
- **Contradiction:** the general rule makes a keyed irreversible
  Process expressible; the sheet, taken literally, would refuse the
  key as an unknown attribute. One of them must yield.
- **Interim behavior (implementation):** the checker admits only
  `effect = idempotent`; `compensable` and `irreversible` are
  explicit deferrals until their companions (`on compensate`, the
  key) are executable — so the contradiction cannot be reached.
- **Resolution direction:** either add `idempotency_key` (CEL
  expression domain) to every configurable connector sheet, or state
  that sheets extend the base attribute set implicitly defined by
  the effect classes.
- **Found by:** external code review (fourth pass), 2026-08-03.

## E-2026-08-03-4 — no canonical byte form for materialized data

- **Where:** [identity.md §3](identity.md), spine key 8 (`data` =
  `[encoding_tag, bytes]`, "opaque, compared as bytes, never
  re-encoded").
- **Gap:** an in-memory (materialized) value has no storage bytes,
  and the spec names no canonical codec for producing them — key 8
  is unprojectable for the most common M0 payload.
- **Interim behavior (implementation):** a materialized value seals
  as its own `tagged_value` deterministic CBOR under `raw` —
  injective and byte-stable, documented at the projection site.
- **Resolution direction:** either bless that rule in §3, or require
  every sealed envelope to carry an encoded representation.
- **Found by:** external code review (fifth pass), 2026-08-03.

## E-2026-08-03-5 — the response closure wants an envelope that does not exist yet

- **Where:** [identity.md §3](identity.md), OUTBOUND key 45
  (`[1, envelope]` for a response closure) vs the ownership rule of
  [sim-05](simulation-semantics.md) and delivery-semantics: the
  dispatcher never mints envelopes — the outcome envelope is born in
  the connector's own transaction, AFTER the closure fact.
- **Contradiction:** the normative closure shape presupposes an
  envelope at close time; the causal model creates it one record
  later. Both cannot hold as written.
- **Interim behavior (implementation):** the closure seals a
  data-only spine (key 8 alone) carrying the raw reply; the full
  envelope appears in the outcome transaction's emission, where it
  is minted.
- **Resolution direction:** either the closure shape becomes
  `[1, reply-data]`, or closures are re-specified to reference the
  outcome transaction's emission by id.
- **Found by:** external code review (fifth pass), 2026-08-03.
