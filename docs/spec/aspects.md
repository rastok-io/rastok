# Aspects — secondary classifications on nodes

> **Status:** normative — under review.
> Defines the generic *aspect* mechanism that powers `Saga`
> ([effects-and-sagas.md](effects-and-sagas.md)) and `Loop`
> ([loop.md](loop.md)). Section 7 lists known open points.

A Rastok node carries up to two kinds of classification in the same `:`
slot : a **trait** (its implementation contract) and zero or more
**aspects** (orthogonal scoping behaviors). Trait and aspect compose with
the `+` operator. The same syntactic mechanism applies to Connectors,
Operators and Graphs.

This document defines the generic mechanism. The two initially
specified aspects (`Saga`, `Loop`) have their own dedicated specs.


## 1. Trait vs aspect

| | Trait | Aspect |
|---|---|---|
| **What it says** | "this node is *an instance of* T" | "this node *behaves with* A" |
| **Cardinality on one node** | at most one | zero or more |
| **Examples** | `HttpClient`, `Postgres`, `Process` (Connector traits) ; `Router`, `Reducer`, `Aggregator` (Operator stdlib aliases) | `Saga`, `Loop` |
| **Required on Connectors ?** | yes | no |
| **Required on Operators ?** | no (optional stdlib alias) | no |
| **Required on Graphs ?** | no (no graph trait exists) | no |
| **Runtime realization** | concrete node type the runtime knows how to execute | a runtime layer that orchestrates the node's lifecycle / control flow |

In short : a trait says **what a node *is*** ; an aspect says **how the
scope around it *behaves***.


## 2. Syntactic shape — `: T1 + T2 + …`

One annotation slot, one separator (`+`), order-insensitive between
aspects (but the trait always comes first when present, for readability).

```rastok
// Trait only
connector lookup : HttpClient { url = "..." ; effect = idempotent }

// Trait + one aspect
connector pay : HttpClient + Loop {
  url     = "..."
  effect  = irreversible
  idempotency_key = meta.app.payment_id
  Loop.variant = {
    kind = measure
    initial = 5
    next = value - 1
    until = value <= 0
  }
  Loop.backoff = { curve = exponential, initial = duration("200ms") }
  on out where meta.ext.http_status < 500
  on out where meta.ext.http_status >= 500 { }
  on exit where data is ConnectFailed || data is TimeoutExpired { }
}

// A saga STEP is not annotated : participation = effect + being inside a `: Saga`
// graph (Saga is a GRAPH aspect). The Connector may still carry Loop :
connector reserve : Postgres + Loop {
  query   = "INSERT ..."
  effect  = compensable
  Loop.variant = {
    kind = measure
    initial = 5
    next = value - 1
    until = value <= 0
  }
  Loop.backoff = { curve = exponential, initial = duration("200ms") }
  on exit where data is ConnectFailed { }
  on compensate {
    idempotency_key = meta.app.order_id + "/unreserve"    // its own operation
    query = "DELETE ..."
  }
}

// Graph with one aspect (no trait)
graph payment : Saga {
  ...
}

// Graph with two aspects — using the `.` boundary sugar (equivalent to `in -> … -> out`)
graph robust_payment : Saga + Loop {
  Loop.variant = {
    kind = measure
    initial = 3
    next = value - 1
    until = value <= 0
  }
  . -> reserve -> authorize -> pay -> .
}

// Operator with one aspect (mono-node intra-transaction loop)
operator refine : Loop {
  Loop.variant = {
    kind = measure
    source = data.remaining_steps
    until = data.error_margin < 0.001
    limit = 100
  }
  on in { data = jq! { .data | step } }
}

// Plain — no trait, no aspect
graph subprocess { . -> a -> b -> . }
operator classify { on in { ... } }
```

The compiler resolves each name in the `:` list against the stdlib
symbol tables : `HttpClient` → trait (Connector), `Router` → trait
(Operator stdlib alias), `Saga` / `Loop` → aspect. Any collision
(e.g. a Connector trait declared more than once in the same list) is a
compile error.


### 2.1 Why one separator, not two

Earlier sketches considered `: Trait with Aspect1 + Aspect2`
(trait/aspect syntactically distinguished). The unified form `: T1 + T2`
was chosen because :

- The trait/aspect distinction is **internal to the type checker** ; the
  developer reads the list as "this node is annotated with these
  classifications".
- The classification happens by symbol lookup anyway, not by syntactic
  position — so encoding it in the grammar costs grammar complexity
  without giving the dev anything they don't already know.
- All declaration forms (Connector / Operator / Graph) share one rule
  instead of three.
- IDE / linter can still render trait and aspect differently
  (highlighting, hover, completion) without grammar support.


### 2.2 What each kind of node can carry

| Node kind | Trait | Aspects |
|---|---|---|
| `connector x : T (+ A)*` | **required** (a stdlib NativeType) | optional, 0..n — **`Saga` excluded** (graph-only aspect; a connector participates via `effect` + lexical scope) |
| `operator  x (: T)? (+ A)*` | optional (a stdlib Operator alias) | optional, 0..n |
| `graph     x (: R \| A)? (+ A)*` | implicit canonical `Graph` ; **role** `App` marks the deployment entry graph (exactly one per deployed unit) | optional, 0..n |

Concretely : a Connector always has an explicit trait (a boundary is never
implicit). An Operator may have a stdlib alias (`Router`, `Reducer`, …) or
carry its implicit canonical type `Operator`. A Graph carries the implicit
canonical type `Graph`, optionally specialized by the **role** `App` (the
third annotation category : trait | role | aspect) and/or aspects.


## 3. Catalogue of aspects — and what is NOT an aspect

Aspects are powerful but they have a cost : each one expands the language
vocabulary, adds a runtime layer to implement and test, and creates a
composition matrix with the others. To avoid aspect-creep, we apply a
strict admission test before adding any new aspect to the stdlib.


### 3.1 Five tests for "is X an aspect ?"

A construct X is admitted as an aspect **only if it passes all five tests**.
If it fails one or more, it is expressed via existing primitives (typed
ports, handlers on typed outcome ports, `instances.key`, provenance, `Router`, …) or via an attribute on
an existing aspect.

| # | Test | Question |
|---|---|---|
| **T1** | **Runtime layer** | Does X require a non-trivial runtime mechanism (state machine, scheduler, manager) that is not already provided by the instance scheduler, the saga manager, the loop manager, the provenance chain, or the trait's own implementation ? |
| **T2** | **Dedicated vocabulary** | Does X introduce clauses, attributes, or hooks that would be illegal or meaningless outside of X's scope ? If the vocabulary is generic (e.g. `timeout = duration("30s")`), X is probably just an attribute of a containing aspect, not its own aspect. |
| **T3** | **Active orchestration** | Does X actively modify envelope flow — re-route, delay, retry, compensate, emit derived envelopes ? If X only observes (audit, trace), it is better expressed as a runtime-wide concern, not a per-node opt-in. |
| **T4** | **Non-redundancy** | Can X be expressed naturally with existing primitives (handlers on typed ports, `instances.key`, `Router`, `durability = global`, etc.)? If yes, X is a pattern, not an aspect. |
| **T5** | **Lexical scope** | Does X wrap a *region* (graph body or node body) and behave as a cross-cutting concern over multiple operations within ? If X parameterizes a single operation, it is an attribute. |

The tests are intentionally strict. They are meant to keep the catalogue
small — aspects are a **scarce resource**, not a default reach.


### 3.2 Initially specified

The two initially specified aspects pass all five tests strongly.

| Aspect | What it scopes | Spec | Passes |
|---|---|---|---|
| **`Saga`** | long-running, compensation-based workflow ; lifecycle = `open → committed / rolling_back → rolled_back / rollback_failed` | [effects-and-sagas.md](effects-and-sagas.md) | T1-T5 ✅ |
| **`Loop`** | bounded tail-recursive iteration for retry, convergence, and counted work; lifecycle = `running → committed / variant_violated / exhausted / selected_base_outcome`; configured by the qualified `Loop.variant = { ... }` attribute | [loop.md](loop.md) | T1-T5 ✅ |


### 3.3 Deferred candidates

These aspects pass the admission test and are recognised as legitimate
candidates. They are **not specified**; listing them here records that
they qualify conceptually without committing an implementation order.

| Candidate | Intent | Composition concerns | Status |
|---|---|---|---|
| **`RateLimiter`** | cap envelope admission rate at a node or region ; `rate`, `burst`, `algorithm`, `overflow = block/error/drop` | Composes with `Loop` (anti-storm on retries) and `Saga` (admission control). `overflow = block` is conceptually a controlled source of backpressure — reuses the runtime's existing channel backpressure. | Deferred; highest-priority candidate because the need is near-universal |
| **`CircuitBreaker`** | fail-fast on consecutive failures ; `closed → open → half_open` state machine, `threshold`, `recovery`, `probe` | Composes with `Loop` (short-circuits retries when open). | Deferred |
| **`Cache`** | memoize external call results by key with TTL | Borderline (T1) — cache state is substantial, closer to a Connector wrapping pattern. Re-evaluate when spec'ing. | Deferred |
| **`Bulkhead`** | concurrency cap (separate pool) ; `max_concurrency`, `queue_depth` | Very close to `RateLimiter` (rate-of-arrival vs. rate-of-concurrency). May merge with `RateLimiter` under a shared `Throttle` aspect ; evaluate when spec'ing. | Deferred |
| **`Hedging`** | race N replicas of a Connector, first response wins | Complex composition with `Saga` (which replica's commit "counts" ?) and `Loop` (race vs. retry confuse). Defer until concrete use case appears. | Deferred until a concrete use case exists |


### 3.4 NOT aspects — better expressed otherwise

These were considered and **rejected** as aspects after applying the
tests. They are better expressed with existing primitives, or do not
belong at the per-node level at all.

| Rejected | Why not an aspect | Express via |
|---|---|---|
| **`Timeout`** | Fails **T2** (generic vocabulary — `timeout = …`) and **T4** (already an attribute of `Saga`, and `Loop` is bounded by its variant). | `Saga { timeout = … }` as an attribute. A node-wide timeout outside any aspect can be a `Timeout` attribute on the node itself, not a kind. |
| **`Fallback`** | Fails **T4** — fully expressible with a typed outcome port and a `Router` on the alternate path. | handler on the selected exit/case + alternate edge wiring. |
| **`Audit`** | Fails **T3** — audit is observation, not orchestration. Fails **T4** — `durability = global` already provides provenance, and a manual audit tee is just an explicit emission. | `durability = global` + an emission on an `audit_out` port wired to a sink. |
| **`Trace`** | Fails **T3** (observation) and **T5** — tracing is a runtime-wide concern, not a per-node opt-in. Activating it for some nodes and not others would create blind spans. | A runtime-wide configuration option (`runtime.tracing.enabled = true`), with `meta.sys.trace_*` populated automatically. |
| **`Validate`** | Fails **T4** — already expressible via typed ports (`input X : SomeSchema`) and checked narrowing. | `input X : Schema` declarations + `as` cast, with its typed refusal routed through the containing contract's ordinary outcome port. |
| **`Mutex`** | Fails **T4** for the common case — a keyed Node already serializes one transaction per logical instance. Cross-address mutex is rare and niche. | `instances.key` for the keyed case. For the rare cross-address case, use a `Postgres` advisory-lock Connector explicitly. |

**Principle** : when a concern can be expressed via existing primitives
or by adding an attribute to an existing aspect, prefer that. Reserve
new aspects for behaviors that genuinely need a runtime layer and a
dedicated vocabulary.


### 3.5 Both Saga and Loop pass — re-validation

For completeness, here is the initial catalogue validated against the
tests :

**Saga**
- **T1** ✅ saga manager : provenance walker + compensation dispatcher + 5-state lifecycle tracker
- **T2** ✅ `on compensate`, `name`, `Saga.timeout`, the saga env entries (`meta.sys.saga_*`) — this *vocabulary* is scope-gated. (**Not** `effect` : effect is mandatory on every Connector, saga or not — effects-and-sagas §2.1.)
- **T3** ✅ re-routes compensation envelopes upstream after an envelope reaches the declared `Saga.rollback` input
- **T4** ✅ no existing primitive expresses bounded compensation chains over arbitrary compensable Connectors
- **T5** ✅ scope = the graph body, wraps multiple steps

**Loop**
- **T1** ✅ loop manager: re-injection scheduler, iteration counter, and
  measure strict-decrease checker
- **T2** ✅ `Loop.variant`, `Loop.backoff`, `measure`, `Loop.value`, and the
  qualified lifecycle hooks — this vocabulary is illegal outside a Loop
  scope (the `meta.sys.loop_*` entries are scope-gated too; envelope.md
  §1.2)
- **T3** ✅ re-applies the body to a passed envelope at every recursive call
- **T4** ✅ no existing primitive expresses bounded self-application over arbitrary regions with a Floyd well-founded measure
- **T5** ✅ scope = the graph body or operator/connector body, wraps multiple recursive calls or a sub-DAG


### 3.6 The tests stay alive

The five tests are not folklore — they are normative. When a future aspect
candidate is proposed, the first action is to apply T1–T5 in writing,
either confirming admission (with the score per test) or rejecting with
the failing test and the alternative expression. This entry is kept in
the candidate's PR / design memo so the rationale is durable.


## 4. Structural properties shared by all aspects

Every aspect, present or future, must obey these rules so the static
analysis and the runtime stay coherent.


### 4.1 Aspect-specific vocabulary is gated by the annotation

An aspect introduces a small set of names (attributes, clauses, hooks).
These names are **only legal** inside a node carrying that aspect ; the
compiler rejects them elsewhere.

```rastok
operator x {
  Loop.variant = {               // ❌ compile error: the qualified
    initial = 100                        //    attribute requires `: Loop`
    next = value - 1
    until = value <= 0
  }
}

operator x : Loop {
  Loop.variant = {               // ✅ legal
    initial = 100
    next = value - 1
    until = value <= 0
  }
}
```

This is *not* duck typing : the kind is declared explicitly, the
clauses follow. Forgetting the annotation does not silently make the
node iterative ; it produces a clear error.


### 4.2 Aspects do not introduce DAG cycles

An aspect may need to re-route envelopes "backwards" (a `Loop` re-injects
on the next iteration, a `Saga` dispatches compensation envelopes to
earlier steps). These back-edges are **runtime mechanisms**, not edges
in the `.rkg` file. The static topology stays a strict DAG.

This mirrors the instance-addressing and provenance mechanisms: declared in the
DSL, orchestrated by the runtime, and invisible at the topology level.


### 4.3 Aspects are bracket-matched by the `{ }` of their scope

Because an aspect always lives on a `graph`, `operator` or `connector`
declaration, its scope is **lexical** — delimited by the `{ }` of the
node body. No marker pairs, no path-walking with a stack : the parser's
own bracket matching is the bracket matching of the aspect.

This is strictly more rigid than open/close marker pairs, which require
path walking and can admit interleaving errors. With lexical scoping,
**interleaving is grammatically
impossible** — you would have to mis-parenthesize the DSL, which does
not compile.


### 4.4 Aspects compose by lexical nesting

A graph annotated `: Saga + Loop` composes both managers. Loop resolves
each bounded application; successful compensable effects become Saga
steps, and a guarded edge explicitly routes a selected terminal case to
`Saga.rollback`. The pair's
rules live in [loop.md §7](loop.md#7-composition-with-saga) and
[effects-and-sagas.md §5.1](effects-and-sagas.md#51-workflow-on-permanent-failure).

Two graphs nested do the same with proper bracketing :

```rastok
graph outer : Saga {
  ...
  graph inner : Loop {
    Loop.variant = {
      kind = measure
      initial = 5
      next = value - 1
      until = value <= 0
    }
    on exit where data is Completed
    ...
  }
  ...
}
```

Outer is a saga ; inner is a loop nested inside it. A selected inner
exit may be wired to the outer aspect's `Saga.rollback` input. The
inner's success is sealed (cannot be reopened by the
outer's rollback — see [effects-and-sagas.md §4.4](effects-and-sagas.md#44-runtime-semantics-sealed-sub-sagas)).


### 4.5 Aspects are runtime layers, not DSL constructs

The DSL declares what each aspect needs (its clauses, attributes,
hooks). The actual orchestration — re-injecting iteration envelopes,
walking the provenance for compensation, observing the lifecycle state
machine — is implemented by **runtime layers** that read those
declarations and execute them.

| Layer | Declared by | Runtime mechanism |
|---|---|---|
| Provenance | `durability = global` | append-only chain of commit records |
| Node instance | optional `instances.key` + `lifetime` | addressed mailbox + FIFO + one active transaction per instance |
| Saga manager | `: Saga` aspect + `effect = compensable` | provenance walker + compensation dispatcher |
| Loop manager | `: Loop` aspect | re-injection scheduler + iteration counter + measure checker |

Each layer is statically declared, deterministically executable, and
visible in the provenance.


### 4.6 Ports contributed by an aspect are ordinary typed ports

An aspect contract may contribute inputs or outputs to the resolved node
contract. Once resolved, those ports use the same direction, effective
`DataType`, wiring, emission and journal rules as every declared port;
an aspect does not gain a private control channel. The qualified spelling
keeps the aspect-owned input unambiguous inside its scope, for example
`Saga.rollback`. An exit exposed by that contract is likewise an
ordinary typed output, and a guarded edge must explicitly select the case
that should reach `Saga.rollback`.

A contributed name that collides incompatibly with a declared or
trait-contributed port is a contract error. The compiler records the
resolved port once; the runtime does not rediscover its aspect origin.


## 5. Multi-type resolution rules

When parsing `name : T1 + T2 + …` :

1. Each `Ti` is a single identifier ; no parameterization (no `Loop<…>`).
2. The compiler classifies each `Ti` via stdlib symbol tables :
   - `HttpClient`, `Postgres`, `Process`, … → Connector trait
   - `Router`, `Reducer`, `Aggregator`, … → Operator stdlib alias
   - `Saga`, `Loop` → aspect
3. Rules per node kind :
   - **Connector** : exactly one trait in the list (mandatory) ; zero or
     more aspects. Trait first by convention (not enforced).
   - **Operator** : at most one stdlib alias (optional) ; zero or more
     aspects.
   - **Graph** : zero traits (none are currently defined) ; zero or more aspects.
4. The same aspect listed twice is a compile error.
5. An aspect listed on a node kind that does not support it is a
   compile error (e.g. `: Foo` where `Foo` is registered as a Connector
   trait but you wrote `graph x : Foo` — graphs accept aspects only).


## 6. Conflict rules — the qualified dotted form

When a trait and an aspect (or two aspects) declare the same attribute name
on one node — e.g. `HttpClient.timeout` (the per-call budget) and
`Saga.timeout` (the completion budget) — the **unqualified name is a compile
error (ambiguous)** and the **dotted qualified form disambiguates**, reusing
the existing attribute-namespace grammar (`jwt.cache_ttl`) — zero new
syntax :

```rastok
connector pay : HttpClient + Saga {
  timeout            = duration("5s")     // 💥 compile : ambiguous — both declare `timeout`
  HttpClient.timeout = duration("5s")     // ✓ the trait's, qualified
  Saga.timeout       = duration("30s")    // ✓ the aspect's, qualified
}
```

The rule is symmetric (either side can be qualified) ; there is **no
priority** (either "trait wins" or "aspect wins" would surprise half the
time). The common no-collision case stays terse — a bare name is legal
whenever it is unambiguous.

**The rule extends to lifecycle triggers** : bare
`on start` / `on stop` bind to the **current Node instance**; an
aspect whose lifecycle differs qualifies its hooks — `on Loop.start { }`,
`on Loop.stop { }` (a loop run ≠ the instance's life). `on compensate` stays
bare : nothing node-level collides with it.


## 7. Limitations and open questions

This specification defines the **shape** of the aspect mechanism. The
points below remain open and must be resolved before an implementation
depends on them.

1. **Generic aspects (`: Loop<HttpResponse>` or similar).** Out of scope
   in the current contract — aspects are non-parametric. May be revisited if a real use
   case appears (e.g. a `Cache<K, V>` aspect).

2. **User-defined aspects.** Currently aspects are stdlib-only (`Saga`,
   `Loop`). A future version may allow apps to define their own aspects
   (a small specialisation framework). Out of scope for the current contract.

3. **Aspect interaction matrix.** The `Saga + Loop` composition is
   spec'd in [effects-and-sagas.md](effects-and-sagas.md). Future aspect
   pairs (e.g. `Cache + Loop`, `RateLimiter + Loop`, `CircuitBreaker + Saga`)
   will need their own composition specs. The default — treating aspects
   as independent layers — works in most cases but not all (Loop's
   re-injection inside a Saga raises the "compensate every call ?"
   question).

4. **Effect rules inside Loop.**
   [Delivery semantics §4.3](delivery-semantics.md) defines the rule: the key names
   the business operation, not the attempt — stable business key = the
   norm, per-attempt key (`iter_count`) = the documented escape for
   partners that consume the key on failure. A per-attempt key exposes
   timeout-with-late-success to double application and must never be the
   default.

5. **IDE rendering of trait vs aspect.** The grammar treats them
   identically. The IDE / linter should render the two visually
   distinctly (color, hover description, etc.) so the dev sees the
   semantic difference at a glance. Out of scope for the language spec.

6. **The word "aspect" itself.** Chosen over `kind` (technical), `region`
   (compiler-flavored) and `mode` (ambiguous with runtime modes). The
   word *aspect* carries a slight AOP heritage which fits the
   "orthogonal cross-cutting behavior" intent. Revisitable if a better
   term emerges.


## 8. Related docs

- [effects-and-sagas.md](effects-and-sagas.md) — the `Saga` aspect : effect
  classes, `on compensate` handler, saga manager, sealed sub-sagas,
  lifecycle.
- [loop.md](loop.md) — `Loop.variant`, `Loop.backoff`, retry,
  refinement, and counted repetition.
- [grammar.md](grammar.md) — the grammar production
  for `type_annotation` and the LL(1) table entries.
- [WHITEPAPER.md §2.5](../../WHITEPAPER.md) — the stdlib alias table
  (traits and aspects).
