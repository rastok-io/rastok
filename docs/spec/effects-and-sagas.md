# Effects and Sagas

> **Status:** normative — under review.
> Companion to [aspects.md](aspects.md) (generic mechanism for trait + aspect
> composition), [loop.md](loop.md) (the `Loop` aspect for retry / iteration),
> and [delivery-semantics.md](delivery-semantics.md). This doc covers
> the **effect classification** of Connectors and the **Saga aspect** that
> orchestrates long-running compensation-based workflows. Section 8 lists
> known open points.

This document is the source of truth for: which actions can be retried, which
must be compensated, which are irreversible — and how Rastok bounds and
orchestrates long-running workflows that mix them. The whole machinery is
**statically verifiable** from declarations alone: no hidden runtime magic, no
unanalyzable cycles, no silent double-effect.


## 1. The fundamental rule : effects live on Connectors

Operators are **pure** — they compute, route, aggregate state inside the graph,
but they do not write to the world. Only **Connectors** bridge the graph to
external systems (HTTP, processes, files, databases, brokers).

Therefore the effect-class machinery described below applies **exclusively to
Connectors**. An `Operator` cannot declare an `effect` ; its actions are by
construction confined to the graph.


## 2. The three effect classes

Rastok uses three classes of side-effect, chosen as the smallest set that
captures saga semantics without sacrificing clarity. Each Connector declaration
states exactly one.

| `effect` | Replay-safe ? | Undoable ? | Where in a saga |
|---|---|---|---|
| **`idempotent`** | yes (replay = same observable result) | n/a (no need) | anywhere ; mandatory after the pivot |
| **`compensable`** | only after a compensating action runs | yes, via the `on compensate` handler | before the pivot |
| **`irreversible`** | no — must be effectively-once | no (can never be undone) | the pivot point of a saga |

Saga-literature gloss (for readers coming from Microsoft / Temporal / Camunda
docs) : `idempotent` ≡ *retriable*, `compensable` ≡ *compensatable*,
`irreversible` ≡ *pivot*.


### 2.1 `effect` is mandatory

Every `connector x : T { ... }` declaration MUST carry an `effect` attribute,
either declared explicitly or inherited from the stdlib type. The compiler
refuses any Connector without one — this is `secure by default` : forgetting to
declare = no retry is allowed (the runtime treats it as irreversible).

```rastok
connector lookup_user : HttpClient { effect = idempotent }
connector reserve     : Postgres   { effect = compensable ; on compensate { … } }
connector pay         : HttpClient { effect = irreversible ; idempotency_key = meta.app.payment_id }
```


### 2.2 Stdlib pre-declares when the type implies the effect

Some Connector types have an inherent effect that no configuration can change ;
the stdlib carries the declaration so the dev doesn't repeat it :

| Connector type | Pre-declared `effect` |
|---|---|
| `SmtpSender` | `irreversible` |
| `Memory` | `idempotent` |
| `Clock` | `none` (read-only) |
| `Calendar` | `none` (pure source) |
| `HttpServer` (inbound) | `none` (pure source) |

For configurable types — `HttpClient`, `Postgres`, `Sqlite`, `Process`,
`Kafka`, `File` (when writing) — the effect depends on the operation and the
dev MUST declare it per instance. An LLM may pre-fill based on conventions
(e.g. POST without idempotency key ⇒ likely `irreversible`) ; the dev signs off.


### 2.3 Companion attributes (statically enforced)

| If `effect =` | Required | Forbidden |
|---|---|---|
| `idempotent` | (nothing extra) | — |
| `compensable` | `on compensate { … }` | — |
| `irreversible` | `idempotency_key = <cel>` | `on compensate { … }` (a pivot cannot be undone) |

A compensation must be **safe to retry** (the saga manager may re-deliver
it — recovery replays with the byte-same key). The compensating
**action** must be idempotent-in-effect, which is satisfied
by EITHER (a) a **delegated** target Connector declaring
`effect = idempotent`, OR (b) an **inline** `on compensate` carrying its own
`idempotency_key` (§3.2) — the partner's dedup provides the idempotence.

**Retry policy is no longer an attribute of `on compensate`.** Bounded
retry on a compensation (or on any external call) is now expressed via the
[`Loop` aspect](loop.md) — see §3.3 below.


## 3. The `on compensate` handler

A compensable Connector reacts to compensation events through an `on compensate`
handler — same shape as the existing `on in`, `on start`, `on stop`, `on tick`
handlers (no new grammar form).

```rastok
connector reserve : Postgres {              // participation = effect + lexical `: Saga` scope
  query  = "UPDATE inventory SET held = held + ?qty WHERE sku = ?sku"
  effect = compensable

  on compensate {
    query = "UPDATE inventory SET held = held - ?qty WHERE sku = ?sku"
  }
}
```

Mental model : the saga manager synthesizes a compensation envelope and
delivers it to the Connector's `in` (with `meta.sys` markers identifying it as
a compensation). The `on compensate` handler runs, does the inverse action,
and emits the result on `out`.


### 3.1 Inline vs delegated compensation

The handler body can perform the compensation **inline** (same Connector type,
just a different query / command) or **delegate** to another Connector by
emitting on `out` :

```rastok
// inline (same system)
connector reserve : Postgres {
  effect = compensable
  on compensate { query = "…" }             // inside a `: Saga` graph
}

// delegated (compensation lives elsewhere — different type, shared across Connectors, …)
connector authorize_card : HttpClient {
  effect = compensable
  on compensate {
    out = { … }                            // emits toward the wired target
  }
}
authorize_card.out -> void_authorization.in   // normal forward edge

connector void_authorization : HttpClient { effect = idempotent ; … }
```

Forward envelopes and compensation envelopes share the same `in`/`out` ports;
the saga manager tags them via `meta.sys` so downstream routing (Router, etc.)
can distinguish them when needed.


### 3.2 Static rules on `on compensate`

- A `compensable` Connector without `on compensate { … }` is a compile error.
- The compensation must be retry-safe : delegated target with
  `effect = idempotent`, or inline handler with its own key (next rule).
- **`E-COMPENSATE-KEY`** : an inline `on compensate`
  on a Connector whose forward path declares `idempotency_key` **must
  declare its own `idempotency_key`** — the compensation is a *different
  business operation* ; reusing the forward key gets the release
  **deduplicated into the hold** by a key-deduping partner (silent loss with
  a clean `rolled_back`). The taught norm names the inverse operation :

  ```rastok
  on compensate {
    idempotency_key = meta.app.booking_id + "/release"
    out = { data = { op = "release" } }
  }
  ```

  (Per-handler attribute override — the same move as `on compensate { query = … }`.
  Wanting the SAME key — cancel-by-same-key APIs exist — is expressed by
  writing it again, visibly.)
- `on compensate` outside any enclosing `: Saga` scope = compile
  **warning** (dead code in all likelihood).


### 3.3 Bounded retry on compensation — the Loop aspect

When a compensation may transiently fail (network blip, downstream
restart), wrap the Connector in a `Loop` aspect — the same mechanism
used for forward retry :

```rastok
connector reserve : Postgres + Loop {        // inside a `: Saga` graph
  query   = "INSERT ..."
  effect  = compensable
  behavior = deterministic
  idempotency_key = meta.app.order_id + "/reserve"
  Loop.variant = {
    kind = measure
    initial = 5
    next = value - 1
    until = value <= 0
  }
  Loop.backoff = { curve = exponential, initial = duration("200ms") }
  on err where err.transient { }             // iterating case : runtime re-injects
  on compensate {
    idempotency_key = meta.app.order_id + "/unreserve"   // its OWN operation
    query = "DELETE ..."
  }
}
```

The Loop policy applies to **both** the forward call and the
compensation handler — the same retry/backoff settings cover both
directions. This is the usual default because transient errors are
transient regardless of direction.

When two distinct policies are required, delegate the compensation to
its own Loop-wrapped Connector — see [loop.md §7.1](loop.md#71-same-node-composition).

An inline `retry = { curve = exponential, … }` attribute is not legal inside
`on compensate { … }`: it would duplicate `Loop` for the same semantics.
There is exactly one mechanism for bounded repetition, used uniformly.


## 4. Long sagas — opt-in, declared with the `Saga` aspect

A **saga** is a long-running, multi-step workflow whose intermediate effects
can be rolled back via compensation. It is **not** an ACID transaction :

| ACID property | In Rastok sagas |
|---|---|
| Atomicity (all-or-nothing) | ✗ each step commits independently ; rollback is via compensation, not undo log |
| Isolation (no intermediate visibility) | ✗ no isolation — concurrent envelopes can observe partial state |
| Consistency (invariants always hold) | ✗ temporary inconsistency during the workflow ; **eventual** consistency |
| Durability (commits survive crashes) | ✓ each step's commit is durable ; saga state is a **fold over the journal** (delivery-semantics §7.1), durable at the saga's scope (`local`/`global`) |

For this reason, the aspect is called `Saga`, **not** `Transaction`
or `Tx` — to avoid misleading ACID associations. The literature term
**saga** is in fact more honest *and* clearer than any "general" synonym.

Sagas are **opt-in** : if no node on a path carries the `Saga` aspect,
the path is not a saga and the saga machinery is dormant (zero overhead).
Many useful workflows are not sagas (e.g. an agent session that just
retries on failure with no rollback needed).


### 4.1 Declaring a saga — the `: Saga` annotation

A saga scope is declared by annotating a node — typically a graph — with
the `Saga` aspect :

```rastok
graph payment : Saga {
  name    = "payment"          // optional ; for observability / audit
  timeout = duration("30s")                // optional ; the saga must complete (commit OR rollback) within this

  // `.` is the graph boundary sugar — equivalent to `in -> … -> out` ; see grammar.md.
  . -> reserve -> authorize -> pay -> .
}
```

The `{ }` of the graph body **is** the saga's scope. Every compensable
Connector traversed inside that scope is tagged with the saga's id in
the provenance ; on permanent failure, the saga manager walks back
through them. No `BeginSaga` / `EndSaga` markers, no path-walking with a
stack — the parser's own bracket matching delimits the saga.

A Connector can also carry the aspect directly when the saga is a
single external call wrapped with effect classification — but the
common case is a graph spanning multiple steps.


### 4.2 Scope = the graph body

```text
graph payment : Saga {

  in ──▶ reserve ──▶ authorize ──▶ pay ──▶ out
           (comp)      (comp)     (irrev)
              │           │
              └── scope of compensation ──┘
}
```

Inside the scope, every commit of a `compensable` Connector is tagged with the
current `saga_id` in the journal's records. On failure, the saga manager uses
these tags to find what to roll back.

Outside the scope — i.e. on paths that have not entered a `: Saga` node —
envelopes carry no `saga_id`, the saga manager is dormant, and there is
**zero overhead**.


### 4.3 Nesting allowed, interleaving structurally impossible

Sagas may be **nested** by lexically nesting graphs :

```rastok
graph outer : Saga {

  . -> step_a -> step_b -> .

  graph inner : Saga {
    . -> step_c -> step_d -> .
  }

  step_b -> inner.in
  inner.out -> step_e
}
```

Because the saga scope is delimited by the `{ }` of the graph body,
**interleaving is structurally impossible** — you would have to
mis-parenthesize the DSL, which does not compile. Open/close marker pairs
would instead require path walking and could be defeated by clever wiring;
they are not part of the model.


### 4.4 Runtime semantics — sealed sub-sagas

When an outer saga contains a nested inner saga :

- Inner **commits** (reaches the end of its `: Saga` graph) → the
  inner saga is **sealed**. Its compensation chain is closed ; its
  effects are definitive at its level ; the outer saga cannot reopen
  it. The sealed inner saga acts as a **local pivot** within the
  outer's flow.
- Inner **fails** → rollback of the inner only. The outer continues and
  receives the error on the relevant branch ; it may handle the error and
  carry on.
- Outer **fails** → rollback of the outer's compensable steps. Already-**sealed**
  inner sagas are **not** retroactively rolled back (the inner has already
  reached its point of no return at its level).

This is the **sub-workflow** model (Temporal-style). Important to surface
because the naive intuition expects a parent saga to roll back its children —
in Rastok, a sealed sub-saga is a **commitment boundary** the parent cannot
cross. If a flow needs the more powerful **XA-style** "inner commit tentative
until outer commits" semantics, it must be modeled as a single flat saga (no
nested `: Saga` graph). True nested-transaction semantics are out of
scope for the current contract.


## 5. The saga manager — a declarative runtime layer

The saga manager is **not** drawn as edges in the `.rkg` file. It is a runtime
mechanism declared *implicitly* by :

- per-Connector : `effect = compensable` + `on compensate { … }`
- per-scope     : `: Saga` aspect on a graph (or any node)

It runs alongside the partition manager and the provenance chain. All three
share the same property : **statically declared, runtime-orchestrated, no
hidden magic**.

| Layer | Declared by | Runtime mechanism |
|---|---|---|
| Provenance | the journal (`durability = local` for single-host recovery ; `global` for cross-host/audit reach) | the causal records + `join_inputs` (delivery-semantics §6) |
| Partition / grain | `per <partition>` | mailbox + FIFO + single-active-instance per key |
| **Saga manager** | `: Saga` aspect + `effect = compensable` | provenance walker + compensation dispatcher |
| **Loop manager** | `: Loop` aspect | re-injection scheduler + iteration counter + measure checker |


### 5.1 Workflow on permanent failure

Rollback triggers when a **permanent failure reaches the saga boundary** —
aligned with the err-bubbling model (delivery-semantics §5) and the
Loop valence rule (loop.md §3) :

```text
0. THE TRIGGER : an unhandled typed err BUBBLES lexically and is intercepted
   by the INNERMOST enclosing `: Saga` — a non-retriable failure, a Loop's
   LoopExhausted { calls, last_error } (its measure exhausted WHILE ERRING — a Loop whose
   until fires after a normal completion exits cleanly and triggers nothing),
   or the saga's own SagaExpired (Saga.timeout — a transaction of the saga
   node, trigger l:timeout, parent = the instance-creating envelope).
   Explicit err wiring CUTS the bubble — a caught err never rolls back.
1. The intercepted saga enters rolling_back.
2. Its sealed compensable steps are compensated in REVERSE order, read from
   the journal fold : each compensation = a sequential transaction of the
   `: Saga` node (trigger l:compensate, port `compensate`, index 0 ; parent =
   the rollback's cause for the first, then the previous compensation's ack —
   envelope.md §3.1). A `: Loop` on the target retries per its policy.
3. All acked → rolled_back ; the saga emits the typed err
   SagaRolledBack { saga_id } on its err port — which BUBBLES OUTWARD : the
   enclosing scope (or app.err) decides — propagate, compensate in turn, or
   handle. A compensation exhausting its Loop → rollback_failed, emitting
   SagaRollbackFailed { saga_id, step } (manual review — §5.3).
   Sealed inner sub-SAGAS of this saga are not re-opened (their own
   compensation already ran or their commit stands — §7.2's nesting rule).
```


### 5.2 Static verifications

On every forward path inside a saga's scope, the compiler
verifies the saga's **shape** :

```text
[ compensable* ]  ──▶  [ pivot ]  ──▶  [ idempotent* ]

where pivot is either :
  - the first irreversible Connector on the path (which carries its idempotency_key), or
  - an explicit `pivot = true` attribute on a compensable Connector (the dev's chosen "point of no return").
```

Compile errors include :

| Error | Why |
|---|---|
| inline `on compensate` without its own `idempotency_key` (when the forward path declares one) | `E-COMPENSATE-KEY` — the release would be deduplicated into the hold; requiring an iteration-specific forward key is not a substitute because it can double-pay on timeout-with-late-success (delivery-semantics §4.3) |
| `compensable` without `on compensate { … }` | the compensation contract is incomplete |
| `compensable` after the pivot | nothing can be rolled back after the pivot |
| `irreversible` before another `irreversible` on the same path inside a saga | only one pivot per saga |
| Compensation target with `effect != idempotent` | compensation may itself need retry |


### 5.3 What we guarantee — and what we don't

Rastok does **not** promise "automatic rollback always succeeds." That would
be a lie in any distributed setting (compensations themselves can fail —
network partitions, downstream outages, programming errors). What Rastok
**does** guarantee is sharper and more honest :

> Effects are **classified** (at compile time), **traced** (via the provenance
> chain), **compensated when possible** (bounded retry via the `Loop` aspect
> on each `on compensate`), and any **failure of compensation becomes visible,
> durable, and treatable** (never silent).

This translates into a small saga lifecycle, with two terminal states for
success and two for various failure modes — all of them durable in the
provenance, and all of them queryable / alertable :

```text
                 entering `: Saga` scope
                         │
                         ▼
                     ┌────────┐
                     │  open  │
                     └────┬───┘
                          │
              ┌───────────┼────────────┐
              │           │            │
        exit `: Saga`  permanent failure   timeout
        scope cleanly     downstream         elapsed
              │           │            │
              ▼           ▼            ▼
       ┌──────────┐   ┌─────────────┐   (treated as
       │ committed│   │rolling_back │    a permanent
       └──────────┘   └──────┬──────┘    failure → rolling_back)
        TERMINAL             │
        (success)            │
                  ┌──────────┴────────────┐
                  │                       │
            all compensations      ≥ 1 compensation
            succeed                exhausts its Loop variant
                  │                       │
                  ▼                       ▼
         ┌─────────────┐         ┌──────────────────┐
         │ rolled_back │         │ rollback_failed   │
         └─────────────┘         └──────────────────┘
          TERMINAL                TERMINAL
          (clean recovery)        (requires manual review)
```

| State | Meaning | Durable in provenance | Operator action |
|---|---|---|---|
| `open` | the saga is in flight | yes | observe |
| `committed` | exited `: Saga` scope successfully | yes | nothing |
| `rolling_back` | downstream failed, compensations in progress | yes | observe |
| `rolled_back` | all compensations succeeded — the saga emits typed err `SagaRolledBack { saga_id }` on `err` (bubbles outward) | yes | nothing |
| `rollback_failed` | at least one compensation exhausted its `Loop` variant (`LoopExhausted`) — the saga emits `SagaRollbackFailed { saga_id, step }` on `err` | yes | **manual review required** |

The crucial point is `rollback_failed` : it is a **first-class terminal state**,
durable, observable, alertable. The runtime never silently drops a failed
compensation ; it marks the saga as `rollback_failed` in the provenance,
emits an audit event, and (configurably) raises an alert / pages on-call /
files an entry in a manual-review queue.

This is what makes the model **honest enough for banking / insurance**. The
guarantee is not "we always recover" — it is "we always **know** what
happened, durably, and we make the unrecoverable cases **visible** instead of
silent."


## 6. The saga's `meta.sys` context — instance-resolved, birth-stamped

Two mechanisms carry saga context :

- **In the CEL env** (inside the lexical `: Saga` scope, resolved from the
  **saga instance**, like partition fields) : `meta.sys.saga_id`,
  `saga_stack`. Always present inside — even while processing the
  scope-creating envelope ; not in the env outside (compile error, even with
  `.?`).
- **Birth stamps** on envelopes minted inside the scope : the same identity
  fields — immutable provenance, journal-visible wherever the wire goes.
- **Journal/observability only** (never in the data plane) :
  `saga_started_at` (wall-clock), the `Saga.timeout` deadline, and the
  lifecycle state (`open | committed | rolling_back | rolled_back |
  rollback_failed` — §5.3) : the state is a **fold over the journal**
  (delivery-semantics §7.1), not an envelope field. Asserting it in a
  scenario means asserting the terminal EMISSIONS (`SagaRolledBack`,
  `SagaRollbackFailed`, `SagaExpired`) — not a meta read.
## 7. Vocabulary — clear primary, saga literature as gloss

Throughout this spec we use **clear domain terms** as primary (devs without
saga-literature background can read them straight) and saga-literature terms
as gloss in commentary and tooling :

| Primary (DSL) | Saga literature (gloss) |
|---|---|
| `effect = idempotent` | retriable |
| `effect = compensable` | compensatable |
| `effect = irreversible` | pivot |
| `on compensate { … }` | compensation handler |
| `: Saga` aspect | saga scope |
| `saga_id` | long-transaction id |

Note : for the saga concept itself, the literature term **saga** is *also* the
clearest term — there is no shorter or clearer general synonym for "long-running,
eventually-consistent, compensation-based workflow." Calling it `transaction`
would be actively misleading (ACID associations). So `saga` is used as both
the technical term and the primary term.


## 8. Limitations, open questions, and fixed interactions

This specification defines the **shape** of effects and sagas. The
items below distinguish open design choices from interactions already
fixed by the normative contracts.

1. **Order of compensations.** LIFO strict (reverse of commit order on the
   provenance chain) versus parallel for independent branches of the DAG when
   the DAG shape proves they don't interfere. *Current default:* LIFO strict
   for simplicity ; parallel can be added as an optimization later, gated by
   a static analysis pass.

2. **Sub-saga semantics.** Chose "sealed independent" (inner commit is final
   at its level — see §4.4). XA-style "inner commit tentative until outer
   commits" is a more powerful but significantly more complex model — out of
   scope for the current contract, revisitable if a concrete use case arises.

3. **Interaction between `saga_timeout` and per-step `Loop` variant.** If a step's
   Loop calls would exceed the remaining saga timeout, should the runtime
   abort the step early to free the saga manager for rollback, or let the
   step finish ? *Current reco:* abort early.

4. **Auto-saga at inbound.** Rejected as default (opt-in only). Could be
   revisited if real cases warrant — e.g. an HTTP service where every request
   should be a saga by convention. The minimum is that the dev can declare it
   explicitly per source.

5. **Cluster-distributed sagas.** When the compensable steps of a saga live
   on different shards / hosts, the saga manager must coordinate cross-host.
   That coordination requires a separate cluster contract; this
   specification only declares the static semantics.

6. **Process lifecycle interplay.**
   ([delivery-semantics.md §7](delivery-semantics.md)). Saga state is a
   **fold** over the journal records of its scope (no dedicated storage) ;
   recovery = fold + outbound triage + resume — a crash mid-saga or
   mid-rollback resumes exactly where the records stop, compensations are
   idempotent on replay (same derived keys), `Saga.timeout` is re-armed from
   the creator record's `committed_at`. A `Process` step's death emits the
   standard terminal envelope on `err`, which **bubbles** into the enclosing
   `: Saga` (delivery-semantics §5) and triggers rollback through the normal
   path ; respawn stays the `restart` policy's business, orthogonal.

7. **Forward vs compensation envelope routing.**
   The marker is `meta.sys.role` — an extensible enum whose value is
   `default` for forward envelopes and `compensate` for compensation
   envelopes. Defined in [envelope.md](envelope.md) §1.1 (Tier 1).

8. **Stdlib compensation Connectors as a pattern.** Should the stdlib provide
   common compensation patterns (`release_resource`, `void_authorization`,
   `refund`, …) as named Connector aliases ? Or does each app define its own ?
   *Current reco:* each app for now ; a community catalog can emerge later.

9. **Interaction with `durability = none`.**
   ([delivery-semantics.md §7.3](delivery-semantics.md)) — the stronger form
   of the first option : **`none` does not exist in a saga's durability
   domain** (a domain restriction, not a combination check — within a
   `: Saga` scope, `durability` ranges over { local, global }). A saga whose
   effective durability resolves to `none` is a compile error
   (`E-SAGA-DURABILITY`) ; `local` (a file, still serverless) is the dev
   path, and simulation tests sagas with no disk at all. Outside sagas,
   `irreversible`/`compensable` under `none` = once-per-app warning
   (process-lifetime guarantees ; loss, never doubling).

10. **Saga inside a `Loop` — rollback granularity across calls.** When a Saga
    is nested in a Loop and call N fails, what is rolled back ? Current
    answer : only call N's saga (sealed sub-saga). Already-sealed sagas of
    calls 0..N-1 are NOT rolled back. May warrant a per-Loop attribute
    `rollback_completed_calls = false | true`; this attribute is not
    currently specified. See
    [loop.md §7.2 and §9](loop.md#72-nested-composition).

Compensation failure is defined by §5.3, with `rollback_failed` as a
first-class terminal state. Saga boundaries are represented by the
`: Saga` aspect rather than begin/end markers. Bounded retry inside
`on compensate` is expressed by the `: Loop` aspect (§3.3), not by a
separate handler attribute.


## 9. Related docs

- [aspects.md](aspects.md) — the generic mechanism that powers `: Saga` and `: Loop`.
- [loop.md](loop.md) — the `Loop` aspect, used for bounded retry on
  forward calls and on compensation handlers.
- [delivery-semantics.md](delivery-semantics.md) — at-most / at-least /
  effectively-once, atomicity contracts, idempotency, unwired-port default,
  process lifecycle. This spec sharpens the `idempotency` bullet and adds the
  compensation / saga story.
- [WHITEPAPER.md §2.5](../../WHITEPAPER.md) — the Operator / Connector
  distinction that makes "effects on Connectors only" a structural property.
- [WHITEPAPER.md §2.7](../../WHITEPAPER.md) — Connector as cycle absorber ;
  the saga manager itself absorbs the apparent "feedback loop" of compensation
  into a forward-only orchestration.
