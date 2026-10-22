# Specification errata

> Known internal contradictions in the normative corpus, awaiting a
> spec-side fix. Rule 1 of [README.md](README.md) applies: when two
> documents disagree, the designated home wins; the entries below name
> the losing text so it is corrected rather than silently reinterpreted.
> An erratum is not a license to improvise — until the fix lands, the
> designated home is the contract.

## E-2026-09-14-1 — type tests lost optional payload witnesses — RESOLVED

- **Where:** `rastok-core/src/expr.rs`, runtime `is`/`as` evaluation.
- **Defect:** deep comparison normalization stripped structural-union branch
  witnesses from nullable fields. A valid `Exited { exit_code, signal: null }`
  consequently failed `data is Exited` when evaluated from a materialized
  value, despite its correct retained type.
- **Resolution:** materialize type-test values without erasing their branch
  witnesses; value-comparison normalization is unchanged. A focused `is`/`as`
  regression and the resident Lab's terminal expectations prove the fix.
  No type tag, source identity or serialized value representation changes.

## E-2026-09-14-2 — nested Session frames lost their owner context — RESOLVED

- **Where:** `runtime/vm.rs` (now `runtime/execution.rs`), admission routing for an existing Session.
- **Defect:** selecting downstream targets without the installed source
  instance re-evaluated a keyed containing Graph from the child frame's new
  metadata. A Process response could enter another conversation instead of
  the one owning that Process, losing its socket-return state.
- **Resolution:** preserve the installed source instance for Session messages;
  new Session/Call/Event admissions still use their pre-commit candidate map.
  The agent-gateway regression proves exactly two remembering/replying
  instances and unchanged records across deterministic re-executions.
  Corrected routes affect only the previously incorrect context; no plan,
  journal format, Contract or non-Session witness changes.

## E-2026-09-14-3 — first Process frame could overtake Session opening — RESOLVED

- **Where:** the hosted resident Process Driver's inbound proposal path.
- **Defect:** a fast child could queue `Opened` and a frame in one poll. The
  frame was proposed before `Opened` reached the core, so the journal correctly
  refused `SessionUnavailable` and the Driver terminated the resource.
- **Resolution:** retain the frame while its opening event is awaiting handoff
  to the core. A deterministic channel test fails before the fix and passes
  after; the two-socket/real-child test exercises the composition. The scripted
  Driver enforces the same handoff for the debugger's one-transition pump,
  covered by stepping/batch comparison and the real-browser regression.
  Publication remains bounded and never-block; no timestamp, acknowledgement
  or journal fact is invented.

## E-2026-09-14-4 — interactive Session mocks reached assertions too early — RESOLVED

- **Where:** `lab.rs`, `Simulator::step_transaction` and `poll_world`.
- **Defect:** absence of ready Execution work was mistaken for quiescence while a
  just-opened scripted Session still had frames to offer. Later multi-frame
  replies could also enter between transactions differently from batch runs.
- **Resolution:** finish ready work before the next scripted world round,
  retain outbound acknowledgements before its inbound frames, and require a
  no-progress poll before declaring rest. No extra scenario action or journal
  fact is introduced. The resident gateway's two scenarios compare full
  canonical batch/step traces; the HTTP source keeps its existing same-record
  regression. Batch goldens are not changed to hide the earlier discrepancy.

## E-2026-08-26-1 — pre-admission HTTP refusals cannot be Call exits — RESOLVED

- **Where:** [stdlib.md §3.1](stdlib.md) publishes `MalformedRequest`,
  `AuthFailed`, and `Overloaded` in `HttpServerExit`, while
  [delivery-semantics.md](delivery-semantics.md) permits an `l:reply` only
  after an inbound request has committed a root and an `InboundCallRef`.
- **Contradiction:** malformed framing, failed authentication, body refusal,
  and listener pressure can reject a request before a typed proposal exists.
  Such a refusal has no Call to close and therefore cannot truthfully emit
  any `HttpServerExit`. Minting a fake root would corrupt provenance merely
  to make the published cases reachable.
- **Interim behavior (H5.1):** the live HTTP/1.1-2 `ack` listener returned the
  portable HTTP refusal directly and commits no causal fact. Only an admitted
  request may emit `Completed` and close through `l:reply`.
- **Resolution:** H5 removes `MalformedRequest` and `AuthFailed` from
  `HttpServerExit`. Every physical binding returns pre-admission protocol
  refusals without minting a root. Only terminal observations of admitted
  Calls inhabit the union; D-2026-08-27-1 records the deliberate scoped
  HttpServer identity move.
- **Found by:** implementation of the first live HttpServer admission seam,
  2026-08-26.

## E-2026-08-07-1 — Tag 3 was reused for the CEL uint domain — RESOLVED

Found by the 2026-08-07 adversarial review, with an executable
counter-example. The canonical encoder gave tag 3 (`long`) both to an
`int` value beyond the 32-bit range and to every CEL `uint`: two
semantically different values sealed to identical bytes (an identity
collision in any projection carrying state deltas), and the plan
package decoder resolved the ambiguity in favour of `uint` — so a
valid program containing `2147483648 + -2147483648` changed meaning
across encode/decode while keeping its semantic hash. **Resolution:**
the registry is extended with tag 23 = `uint` (identity.md §3,
D-2026-08-07-2); tag 3 means `long` only; the plan package moves to
format version 2 with a widened checksum (D-2026-08-07-3); boundary
and execution-equivalence round-trip tests pin the fix.

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

## E-2026-08-03-2 — unwired data outputs: warning or error? — SUPERSEDED

> **Resolved in the M1.3 target by D-2026-08-14-1.** There is no
> name-based `err` exception. Every resolved output carries an explicit
> `UnwiredPolicy`; B5 landed the two exact policies (`RequireWire` and
> `RootSink`) in plan data. Forwarding/discard remain ordinary edges.
> **D-2026-09-05-1 supersedes that resolution for v16:** both policies and
> every implicit sink disappear. A productible output requires an explicit
> disposition, while a re-derived impossible output is exempt and warned.
> The paragraphs below remain the v15 history, not the live target rule.

- **Where:** [grammar.md](grammar.md) (wiring prose) vs
  [delivery-semantics.md](delivery-semantics.md) (routing rules).
- **Contradiction:** grammar.md calls an unwired non-`err` data
  output a *warning*; delivery-semantics.md treats an unrouted data
  emission as an *error*. Both agree `-> nothing` is the explicit
  discard and that `err` bubbles to the root sink.
- **Resolution:** `RequireWire` keeps `W-UNWIRED-OUTPUT`, so exploratory
  graphs still compile; an accepted warning leaves the emission in the
  journal/observation surface but creates no downstream delivery.
  `RootSink` additionally publishes an unclaimed emission at the root sink.
  The resolved row, never the spelling `err`, selects the behavior.
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

## E-2026-08-04-1 — the bare-meta alias collides with custom namespaces

- **Where:** [grammar.md](grammar.md) (`meta.<field>` ≡
  `meta.app.<field>`, stated without exception) vs
  [envelope.md](envelope.md) (custom writable namespaces addressed
  `meta.<namespace>.<field>`).
- **Contradiction:** taken literally, the alias makes every custom
  namespace unreadable by its own path — `meta.audit.origin` would
  resolve to `meta.app.audit.origin`. Both rules cannot be absolute.
- **Interim behavior (implementation):** at the projection root a
  custom NAMESPACE name shadows the app alias; the shadowed app
  field stays reachable at its unambiguous `meta.app.<field>`
  address. Tested.
- **Resolution direction:** scope the alias — `meta.<field>` reads
  `meta.app.<field>` only when `<field>` names no declared custom
  namespace — or drop the bare alias entirely and require the
  explicit form.
- **RESOLVED (2026-08-10):** the second branch, taken to its end by
  D-2026-08-10-1 — the bare alias AND custom namespaces are both
  removed. Metadata has exactly three zones (`sys`/`app`/`ext`);
  every path is explicit; the collision is unrepresentable. The
  interim shadowing projection dies with the model.
- **Found by:** external code review (seventh pass), 2026-08-04.

## E-2026-08-04-2 — the timer ref of key 37 needs its own recipe

> **Status: resolved, 2026-08-06.** The delivery reference and its
> `0x20` domain are formalized in envelope.md §2; `call_ref` now
> derives from it; identity.md §3 seals the causing edge (OUTBOUND
> key 47) and names the timer ref. The interim behavior below is the
> history of the gap.

- **Where:** [identity.md §3](identity.md), `timers` sub-form: `ref`
  is described as "the obligation's causal id (a 16-byte
  envelope/instance id)".
- **Gap:** a call-budget obligation is not an envelope: one delivered
  envelope fanned out to two budgeted edges is two obligations, and
  an envelope id cannot name them apart. No normative recipe exists.
- **Interim behavior (implementation):** the obligation's identity is
  `derive(0x20, canonical_edge_text, envelope_id)` — one obligation
  per receiving edge, two ports of one connector included. Domain
  `0x20` extends the id-domain registry; the recipe is pinned
  byte-for-byte in the id tests.
- **Resolution direction:** formalize the BudgetRef recipe and the
  `0x20` domain in identity.md §2/§3, or re-key `timers` by an
  identity the spec already owns.
- **Found by:** external code review (eighth pass), 2026-08-04.
