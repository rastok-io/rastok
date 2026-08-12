# The Execution Model — mailboxes, ordering, time, and the two planes

> **Status:** normative — under review.
> Companion to
> [envelope.md](envelope.md) (identity, journal-relative contract) — this doc
> pins the rules envelope.md deferred to the execution model : the
> `emission_index` rule, the mailbox-FIFO ordering contract, the dated-events
> queue, and the supervision vocabulary.
> Reference simulations 1–2 exercise mailbox-per-key ordering and the
> virtual-clock contract.

The question this doc answers : **when a graph runs, what happens exactly, in
what order — and what can be reproduced ?**


## 1. The unit of work — the node transaction, run-to-completion

A **node transaction** consumes one triggering envelope at one node and runs
the matched handler to completion : `f(state, input) → state', emissions`.
"Transaction" is meant literally, not as a metaphor — **A**tomic (one commit,
all-or-nothing : the transaction record, delivery-semantics), **C**onsistent
(types + guards), **I**solated (one mailbox, one worker at a time = serialized
per key), **D**urable per the `durability` scope. *(Other traditions call
this unit a Petri* transition firing *or an actor* turn *; Rastok says
transaction because that is what it does.)*

**Naming rule** : bare "transaction" in Rastok docs ALWAYS means the
node-level unit. Any wider scope must be qualified — *graph transaction*,
*cascade transaction* — terms **reserved, not specified** (nothing wider
exists in v0.x ; beyond one node it's the journal, beyond the graph it's the
saga). Shorthand in diagrams and tables : `tx`.

The contract is **as-if-run-to-completion (as-if-RTC)** :

- Observable semantics : for a given partition key, transactions execute one at a
  time, each *as if* uninterrupted, in arrival order.
- Runtime freedom : underneath, the runtime may suspend on I/O, work-steal,
  move keys across workers, or speculate — anything, as long as the observable
  behavior is identical to the one-at-a-time execution. (The same pact as the
  C compiler's as-if rule and database serializability.)

Within a transaction, statements run in **handler-text order** (the imperative
intra-transaction model of the DSL). This order is what makes emission identity
derivable (§4).


## 2. M:N — a mailbox per key, a bounded pool of workers

Not a thread per key (100 000 sessions would kill the OS). One **mailbox per
partition key**, and a small worker pool :

```
   incoming envelopes
        │
        ▼
  ┌─────────────────────────────────────────┐
  │  mailboxes (one per key)                │
  │  session=alice   [ e1, e4 ]   ◄── FIFO  │
  │  session=bob     [ e2 ]                 │
  │  session=carol   [ e3, e5, e6 ]         │
  └─────────────────────────────────────────┘
        ▲ ▲ ▲
   ┌────┴─┴─┴────┐   a worker takes ONE mailbox, processes ONE
   │ W1 W2 W3 W4 │   envelope (one transaction), releases the mailbox.
   └─────────────┘
```

**The golden rule : a mailbox is never held by two workers at once.** Per key,
envelopes are processed one at a time, in mailbox order — which is what makes
`var` state safe without locks, and per-key ordering structural. (This is the
Orleans-grain / Kafka-Streams-task / Restate-virtual-object lineage ; Restate's
virtual objects validate the same conviction independently.)

Async I/O releases the worker (the mailbox stays claimed by the suspended
transaction) ; CPU-heavy work (large `jq`, future `sql`) runs on a separate compute
pool with back-pressure. Both are invisible under as-if-RTC.

A node without `per` is a singleton : one implicit mailbox.


## 3. Ordering — what is guaranteed, what is journaled

```
GUARANTEED : within one mailbox, FIFO — arrival order is processing order.
NOT DEFINED: order across mailboxes (not observable : no shared state).
```

**Joins.** `on (a, b)` buffers early arrivals in partition state ; the transaction
triggers when presence completes, and the **triggering envelope is the last
arrival in mailbox-FIFO order** (arbitration A3). Its port is the transaction's
trigger ; it is the parent of the join's emissions (envelope.md §3).

**The honest part.** On a non-linear graph, the arrival order at a mailbox fed
by two branches is a **race** (a timed merge) : same graph, same inbound, two
runs can interleave differently. Rastok does not pretend otherwise — the
contract is **journal-relative replay determinism**
([envelope.md §2.0](envelope.md)) :

```
LIVE    races happen ──► the per-hop journal records arrival orders
REPLAY  the journal is re-read ──► no race re-runs ──► identical run
SIM     the fixture IS the journal ──► deterministic every run
```

The run is a pure function of `(graph, journal)`. Nothing else in this doc
weakens or strengthens that contract ; everything below feeds events *into*
it.


## 4. Emission identity within a transaction — the `emission_index` rule

envelope.md §2 derives every id as
`blake3( node_path ‖ trigger ‖ parent_id ‖ emission_port ‖ emission_index )`
and defers the index rule here. The rule :

- Emissions occur in **handler-text order** (§1), which is deterministic.
- **`emission_index` counts emissions per output port, per transaction, starting
  at 0**, in that text order.

```
on in {
  out = { … }                   → ("out",   0)
  out = { … }                   → ("out",   1)     // second emit, same port
  (audit, alert) = (m, m)       → ("audit", 0), ("alert", 0)   // broadcast: per-port counters
}
```

Conditional emissions count only when they execute — the index reflects the
*actual* emission sequence of this transaction, which is replay-identical because
the transaction's inputs are (journal-relative) identical.


## 5. The two planes — and the dated-events queue

The **data plane** (the graph) never reads a wall-clock and never observes the
external world directly. The **observation/supervision plane** does both — and
everything it decides enters the graph as a **journaled event** :

```
  ┌──────────────────────────────────────────────────────┐
  │ OBSERVATION / SUPERVISION PLANE     (non-deterministic)│
  │ watches : wall-clock, per-call chronos, resource      │
  │ liveness. decides : tick, timeout, eviction, expiry.  │
  └───────────────┬───────────────────────────────────────┘
                  │ every decision = a journaled event
                  ▼
  ┌──────────────────────────────────────────────────────┐
  │ DATA PLANE = the graph              (deterministic    │
  │ given the journal — §3)                               │
  └──────────────────────────────────────────────────────┘
```

Concretely, the supervision plane maintains **one queue of dated events** :

| Dated event | Armed by | Delivered as |
|---|---|---|
| clock tick | `Clock` (`system_clock` incl.) | a root tick envelope (inbound) |
| per-call timeout | a Connector's `timeout` (§6) | `TimeoutExpired` on the Connector's `err` |
| join eviction | a join's `within` / `last` (§7) | `JoinExpired` on the Operator's `err` |
| partition lifetime expiry | `lifetime = …` | the instance's `on stop` (parent = **the envelope that armed the deadline** — last activity for idle-ttl, the creator for fixed/window) |
| loop backoff delay | `Loop.backoff` ([loop.md](loop.md) §4) | the re-injected call (trigger `l:next`) |
| saga completion budget | `Saga.timeout` ([effects-and-sagas.md](effects-and-sagas.md)) | a transaction of the `: Saga` node — trigger `l:timeout`, parent = the instance-creating envelope ; emits typed err `SagaExpired` (bubbles) and the instance enters `rolling_back` |

In production the queue is driven by wall-clock ; in simulation, by the
fixture (§8). **The delivery contract — order, causality, identity — is
identical in both modes** ; that identity is the foundation of two-mode
equivalence. Replay re-reads the journaled events ; it never re-observes and
never re-runs a race.

Real times (when a chrono was armed, when it expired) live in the journal
(`committed_at`) and the observability spans — **never** on the envelope and
never readable from the data plane (the `created_at` lesson, envelope.md §8).


## 6. Per-call supervision — `timeout` on Connectors

### 6.1 What is bounded

`timeout = duration("30s")` on a Connector is a **per-call budget** : armed **at delivery
into the Connector's mailbox** (not at transaction start — the budget
deliberately includes queueing : a backlogged Connector can time out calls it
never began, and the call-closed rule applies identically), cleared when the
*corresponding* response is emitted. Correspondence is the Connector's own business — its
internal correlator knows what "a call" is in its protocol (HTTP
request/response, JSONL id matching for a `Process`). The supervision plane
supplies the clock ; the Connector supplies the notion of a call.

Not bounded by `timeout` (covered by the observability contract) :
**inactivity** ("no output for 5 min") and **liveness/health** ("is
the process alive ?") — resource-level concerns, not flow-level ones ; they
have no impact on graph determinism.

### 6.2 Expiry — the event

```
supervision : the chrono of call e42 expired
     │  journals the decision (replayable)
     ▼
codex.err ──► typed error envelope TimeoutExpired
                parent_id = e42          (the cause — the timed-out request)
                trigger   = l:timeout    (emitted on behalf of the Connector node)
                err kind  = TimeoutExpired ∈ the Connector's static error set
```

*(`l:timeout` is general : it fires for ANY declared budget's expiry, with
parent = **the envelope that armed the budget** — the request for a call, the
instance-creating envelope for `Saga.timeout`. One trigger, one rule.)*

No new handler form : the graph reacts by ordinary wiring
(`codex.err -> triage`, guards on `err.kind`). The typed `err` channel already
carries "the Connector's declared failure modes" — `TimeoutExpired` joins that
set.

### 6.3 The call-closed rule (anti-ghost)

**A timeout closes the call, definitively.** If the response arrives late, the
Connector's correlator discards it and the discard is **journaled** (`late
response, discarded` — visible to observability, invisible to the graph). One
call = exactly one outcome : a response or a timeout, never both.

### 6.4 The instance — `on_timeout`, orthogonal to `restart`

A call-level failure is not automatically a resource-level failure. Two knobs,
one concern each :

```rastok
connector codex : Process {
  mode       = service
  timeout    = duration("30s")
  on_timeout = bypass               // the INSTANCE's fate on expiry : bypass (default) or kill
  restart    = { … }                // respawn policy after any instance death (a struct — D-2026-08-12-1)
}
```

The fragment omits unrelated required `Process` attributes.

- **`bypass` (default)** : the call fails (§6.2-6.3), the instance lives ;
  later calls proceed. A timeout is a flow fact, not a death sentence.
- **`kill`** : the supervision kills the instance → the **standard death
  path** (WHITEPAPER §2.7 : terminal envelope with the outcome, the implicit
  `run_id` partition ends — its `on stop` is the cleanup hook) → the `restart`
  policy alone decides respawn.

Compositions, not new mechanisms : *restart-on-timeout* =
`on_timeout = kill` + `restart = { … }` ; *die-without-respawn* =
`kill` with no restart policy. A wedged resident process that must restart
uses `kill` (fresh `run_id` plus its protocol-specific resume operation);
the conservative default stays `bypass`.

### 6.5 Attribute-name collisions — the qualified form

`Saga` already owns a `timeout` (its completion budget). When a trait and an
aspect (or two aspects) declare the same attribute name on one node, the
**unqualified name is a compile error (ambiguous)** and the **dotted qualified
form disambiguates** — reusing the existing attribute-namespace grammar
(`jwt.cache_ttl`), zero new syntax :

```rastok
connector pay : HttpClient + Saga {
  timeout            = duration("5s")     // 💥 compile : ambiguous — both declare `timeout`
  HttpClient.timeout = duration("5s")     // ✓ the per-call budget
  Saga.timeout       = duration("30s")    // ✓ the saga completion budget
}
```

The rule is symmetric (either side can be qualified) ; the common
no-collision case stays terse. The general namespace and collision rules are
defined in [aspects.md](aspects.md) §6.


## 7. Join abandonment — `JoinExpired`

A buffered join partner may never arrive. The join contract bounds the wait —
`within duration("15m")` (time window) and `last 100` (count bound), evicted
**lazily** (checked at arrivals and at dated events, never a background
timer). **The `within` chrono is armed when an envelope is first buffered**
(per buffered envelope) and cleared when presence completes and the transaction
consumes it. Eviction cannot be a silent drop; the rule is:

```
eviction of a JOIN buffer (within elapsed OR last overflow)
     │
     ▼
settle.err ──► typed error envelope JoinExpired
                 parent_id = the evicted envelope   (the cause)
                 trigger   = l:evict
                 err kind  = JoinExpired ∈ the Operator's static error set
```

The graph decides by wiring — compensate, alert, DLQ via `Memory`, or
`-> nothing` if abandonment is genuinely normal (then it is *written*, not
silent).

**The boundary that avoids the false friend** : this rule applies to **join
buffers only** (an envelope that awaited presence and never got it = a lost
fact → err). **Window/aggregate eviction is normal semantics** — a closing
window emits its aggregate through `on stop` (§9) ; leaving a window is life,
not failure.


## 8. Virtual time — the `advance` contract

> `advance` belongs to **simulation** — virtual time
> only. An `experiment` (simulation-semantics.md §10) runs the real App in
> real time : `advance` inside it is a compile error ; observation waits
> are real and bounded (profile knob).

In simulation the supervision plane is replaced by the fixture, and time is
**discrete** : `advance duration("2h")` costs microseconds of real time — it moves a
counter and delivers what was due, in order. No real clock, no sleep,
anywhere.

Two principles, then the sweep :

```
P1. COMPUTATION TAKES ZERO VIRTUAL TIME.
    Draining mailboxes happens with the clock STOPPED. Only explicit
    delays (timeout, within, backoff, lifetime, tick) carry dates.

P2. TIME ONLY ADVANCES WHEN ALL MAILBOXES ARE EMPTY.

advance Δ :                     (target = t + Δ)
  repeat :
    drain all mailboxes until empty                  ← t does not move
    if a dated event ≤ target exists :
        t := date of the EARLIEST one ; deliver it   ← one at a time, date order
    else :
        t := target ; done
```

```
t=0   inject(msg) ──► drain : msg → route → codex.in   (chrono armed : expires t=5)
advance duration("30s") (target 30) :
  earliest ≤ 30 ?  t=5  TimeoutExpired(codex) → deliver → drain (triage → alert)
  earliest ≤ 30 ?  t=10 clock tick            → deliver → drain (stats emits)
  earliest ≤ 30 ?  none → t := 30. done.
expect …  ──► evaluated on the STABLE state at t=30
```

Edge rules :
- **Simultaneity** (two events with the same date) : deterministic tie-break =
  **arming order** (first armed fires first).
- **Every fixture step drains** : `inject`, `advance`, `expect` each complete
  on a stable (empty-mailbox) state — an `expect` never reads an in-between.
- A cascade may arm new dated events ; if they fall ≤ target they are swept in
  turn, in date order.

In production the same queue runs against wall-clock ; the delivery contract
(§5) is identical — which is exactly what makes a fixture able to *play the
journal's role*.


## 9. Windows — the minimal forward-compatible contract

Window execution is deferred, but its forward-compatible contract is
defined here so it can land without changing the model. A window in
Rastok is **not** a separate mechanism — it is a
**partition-lifetime mode** :

| Family | Shape | One envelope belongs to | Rastok | Status |
|---|---|---|---|---|
| **Session** | elastic, closed by silence | 1 window | `lifetime = duration("2h")` (idle-ttl) | specified |
| **Tumbling** | fixed boxes, end to end | 1 window | `lifetime = window duration("5m") tumbling` | specified here |
| **Sliding** | fixed, overlapping | **N windows** | `lifetime = window duration("5m") sliding` | declared; dispatch semantics deferred |

The four lines of the contract :

1. **Processing-time, period.** An envelope belongs to the window **open at
   its arrival** (per-partition arrival order — the §3 contract). Corollary :
   *"late events" do not exist* — in processing-time there is always an open
   window for you. The watermark machinery is out of the model.
2. **Event-time is a deferred, additive layer** : windowing on
   `data.occurred_at` would be an *extra clause* ; nothing here changes.
3. **Tumbling boxes are epoch-aligned — in both modes** : boundaries at
   `epoch + k·width`. In simulation, civil time = the scenario's `origin` +
   the virtual offset (`origin` defaults to the epoch, simulation-semantics
   §5.3), so the same events fall in the same windows in both modes.
   Deterministic ; two runs cut identically ; never aligned to the first
   event.
4. **Sliding is declared, not yet spec'd.** Overlap means one envelope maps to
   N live instances — a real multi-instance dispatch question, treated with
   the `Window` alias. Nothing here forbids it.

Window close = lifetime expiry = a dated event (§5) → the instance's `on stop`
emits the aggregate (parent = the closing tick, envelope.md §3.1). Normal
semantics — never `JoinExpired` (§7).


## 10. Trigger vocabulary

Two lifecycle triggers belong to the `l:` registry
([envelope.md](envelope.md) §1.1/§3.1):

| Transaction | `trigger` | `parent_id` (the cause) | Emitter (`node_path`) |
|---|---|---|---|
| Call-timeout expiry | `l:timeout` | the timed-out request envelope | the Connector node |
| Join-buffer eviction | `l:evict` | the evicted envelope | the Operator node |

Both follow the aspect-carrier principle generalized : the supervision plane
never emits in its own name — emissions are attributed to the graph node they
concern. The recipe keeps zero out-of-graph emitters.


## 11. Non-goals of this doc (deferred, with homes)

- **Scheduler implementation** (tokio, work-stealing, compute pool sizing) —
  implementation detail constrained by this model.
- **Inactivity & liveness monitors** — observability concern.
- **Cluster ordering / cross-host anything** — deferred.
- **Event-time & watermarks** — deferred and additive (§9).
- **Sliding-window dispatch** — deferred with the `Window` alias.
- **Hot-key mitigation** — deferred.


## 12. Conformance scenarios

Reference simulations 1–2 must exercise date order, the arming-order
tie-break at equal dates, zero-duration computation, time advancement
only on empty mailboxes, and byte-identical per-key traces across runs.


## 13. Related docs

- [envelope.md](envelope.md) — identity recipe, journal-relative contract,
  the `l:` trigger registry, parent rules.
- [effects-and-sagas.md](effects-and-sagas.md) — `Saga.timeout` (the
  completion budget this doc's §6.5 disambiguates), compensation triggers.
- [loop.md](loop.md) — `Loop.backoff`
  delays as dated events ; `variant`/`until` (data-driven, orthogonal to
  wall-clock supervision).
- [aspects.md](aspects.md) — §6 conflict rule as applied by §6.5.
- [grammar.md](grammar.md) — `timeout` / `on_timeout`
  attributes, qualified dotted attributes.
