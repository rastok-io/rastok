# The Execution Model — mailboxes, ordering, time, and the two planes

> **Status:** normative. The Node-owned instance model in
> [instances.md](instances.md) supersedes the unreleased P0–P3
> named-partition implementation. Categorical plan-package v11 retains the v9
> realigned owner tree and instance policies. R3 routes every delivery through
> one hierarchical instance reference shared by journal, mailbox, state,
> calls, timers, lifecycle, and cursors. R4 executes persistent, idle, fixed,
> single-pass, and processing-time tumbling lifetimes with journal-derived
> generations. R5 executes explicit bounded Closing, successor-generation
> admission, cancellation, settlement, terminal `OutcomeUnknown`, and
> inner-first Graph closure. R6 executes nested composition and bounded joins;
> R7 closes the one-worker slice and proves physical Worker assignment is not
> a causal input. D-2026-08-25-1 adds the M1.6 distinction between Event,
> Call and Session exchanges; H1 makes §6.6's generic session fold executable,
> while concrete network contracts remain gated by H2.
> Companion to
> [envelope.md](envelope.md) (identity, journal-relative contract) — this doc
> pins the rules envelope.md deferred to the execution model : the
> `emission_index` rule, the mailbox-FIFO ordering contract, the dated-events
> queue, and the supervision vocabulary.
> Reference simulations 1–2 exercise mailbox-per-instance ordering and the
> virtual-clock contract.

The question this doc answers : **when a graph runs, what happens exactly, in
what order — and what can be reproduced ?**


## 1. The unit of work — the node transaction, run-to-completion

A **node transaction** consumes one triggering envelope at one node and runs
the matched handler to completion : `f(state, input) → state', emissions`.
"Transaction" is meant literally, not as a metaphor — **A**tomic (one commit,
all-or-nothing : the transaction record, delivery-semantics), **C**onsistent
(types + guards), **I**solated (one instance mailbox, one worker at a time),
**D**urable per the `durability` scope. *(Other traditions call
this unit a Petri* transition firing *or an actor* turn *; Rastok says
transaction because that is what it does.)*

**Naming rule** : bare "transaction" in Rastok docs ALWAYS means the
node-level unit. Any wider scope must be qualified — *graph transaction*,
*cascade transaction* — terms **reserved, not specified** (nothing wider
exists in v0.x ; beyond one node it's the journal, beyond the graph it's the
saga). Shorthand in diagrams and tables : `tx`.

The contract is **as-if-run-to-completion (as-if-RTC)** :

- Observable semantics : for one logical Node instance, transactions execute one at a
  time, each *as if* uninterrupted, in arrival order.
- Runtime freedom : underneath, the runtime may suspend on I/O, work-steal,
  move keys across workers, or speculate — anything, as long as the observable
  behavior is identical to the one-at-a-time execution. (The same pact as the
  C compiler's as-if rule and database serializability.)

Within a transaction, statements run in **handler-text order** (the imperative
intra-transaction model of the DSL). This order is what makes emission identity
derivable (§4).

An Operator is pure with respect to the external world, but its evaluation can
still refuse (for example, a dynamic narrowing has no match or encoded data
cannot be materialized). Such a `Fault` is a supervision fact, not a graph
value: the transaction commits neither its input cursor, state nor emissions,
and the VM returns the fault to its caller. The conventional output name
`err` is never selected automatically. When rejection is expected domain
behavior, the handler expresses it explicitly and emits a value on a declared
output — conventionally `err`, giving the simple shape `I -> Result<R,E>`.
Catastrophic host failure such as an allocator abort is outside this typed
contract; the runtime does not pretend every hardware/process failure is
recoverable in-process.


## 2. M:N — a mailbox per logical instance, a bounded pool of workers

Not a thread per key (100 000 sessions would kill the OS). One **mailbox per
logical Node instance**, and a bounded worker pool:

```
   incoming envelopes
        │
        ▼
  ┌─────────────────────────────────────────┐
  │  mailboxes (one per instance reference) │
  │  session=alice   [ e1, e4 ]   ◄── FIFO  │
  │  session=bob     [ e2 ]                 │
  │  session=carol   [ e3, e5, e6 ]         │
  └─────────────────────────────────────────┘
        ▲ ▲ ▲
   ┌────┴─┴─┴────┐   a worker takes ONE mailbox, processes ONE
   │ W1 W2 W3 W4 │   envelope (one transaction), releases the mailbox.
   └─────────────┘
```

**The golden rule : a mailbox is never held by two workers at once.** Per
instance, envelopes are processed one at a time, in mailbox order — which is
what makes Node-local `var` state safe without locks and instance ordering
structural. (This is the
Orleans-grain / Kafka-Streams-task / Restate-virtual-object lineage ; Restate's
virtual objects validate the same conviction independently.)

Async I/O releases the worker (the mailbox stays claimed by the suspended
transaction) ; CPU-heavy work (large `jq`, future `sql`) runs on a separate compute
pool with back-pressure. Both are invisible under as-if-RTC.

A Node without `instances` is a singleton within its enclosing Graph
instance. A keyed Node has one mailbox per canonical key at each enclosing
address. A `var` is private to the declaring Node, so no graph-wide lock or
cross-mailbox mutable state exists.

### 2.1 Instance addressing and lifecycle

The M1 reference machine is the physical one-worker case of the model above.
It does not promise order across addresses, even though one deterministic pump
makes an order reproducible in that implementation.

An edge crossing a keyed owner is decided before its producer commits. The
`instances.key` expression sees the complete immutable entering envelope —
including `meta.sys` — and immutable values from enclosing lexical scopes.
The owner body is entered only after address selection, so its own `val`s
cannot participate. The producer reserves destination capacity and records
the canonical target instance reference `(address, generation)`. Delivery and
recovery consume that recorded fact; neither re-routes by evaluating the
expression after commit. The hierarchical address selects placement; its
journal-derived generation forms the instance reference that indexes mailbox,
derived immutable instance bindings, Node-local state, open calls, lifecycle,
armed events, and consumer cursor. Type checking follows the ordinary total
port solver and creates no second inference mechanism.

When a route enters several nested owners, selection is ordered outermost to
innermost. Each selected owner reference and its purely evaluated instance
bindings become the immutable enclosing context for the next selection. A
route may not skip an owner boundary or evaluate a child against provisional
mutable state. The final routed target carries one instance reference; the
preserved owner tree and lifecycle records retain its enclosing chain.

Admission of a delivery into an absent address starts a new **instance**:

1. reserve capacity and the zero-based generation derived from journal history
   for that address, then seal that instance reference in the producer route;
2. purely evaluate and validate per-instance bindings; failure consumes
   nothing, starts nothing, and performs no Driver effect;
3. commit `l:start` with the admitted envelope as cause and the reserved
   instance reference;
4. commit the envelope's port transaction only after the start;
5. arm any lifetime deadline with that generation;
6. treat an event carrying an older generation as a harmless stale
   observation.

An effective expiry requests Closing for that instance. Every finite owner
whose subtree contains `Process` carries an explicit positive
`closing = { drain_for, settle_for }` policy. New external admission is routed
to the exact successor generation; already admitted internal work and
Connector-call obligations remain old-generation work. Every admitted
Connector call retains its terminal `exit`, and `on stop` runs only after
every required terminal fact. State is reclaimed only after the stop
transaction commits. The waiting successor then starts with clean state and
cannot see the closed generation.

Mailbox reservation plus the producer's route commit is the admission point.
Reservation returns the instance reference that the route seals. A route that
wins before the committed `l:closing` fact remains old-generation drain work
even if an inter-shard transport arrives physically later. A post-close route
commits to the successor's ordinary bounded mailbox and waits there until the
predecessor stops. Scheduler or network order can therefore never leak an old
delivery into new state, and no second waiting queue exists.

A committed reservation consumes the referenced generation even if no worker
ever polls its envelope. Recovery must complete the corresponding `l:start`,
or bounded Closing must retire that generation while sealing the residual
delivery's consumed cursor in its stop transaction; it may never recycle the
ordinal while a committed route still names it. In a future cross-shard implementation, reservation and route
publication may use a protocol, but a crash may not expose one without a
recoverable path to the other.

Every selected owner, persistent included, records one lazy `l:start` before
its first port transaction. `on start` and `on stop` are ordinary handlers in
an executable Node's handler table and may emit ordinary typed outputs; Graph
lifecycle records execute at R4 while Graph handler lowering remains a named
frontend gap. A Graph close requests inner-first closure of live descendants
created in that Graph generation. A non-root child's `l:start` record carries
its enclosing Graph's complete instance reference, so the lifecycle fold
distinguishes two successive parents at the same logical address. The journal
refuses a start under an inactive parent and a parent stop with an active
child. The close request itself is a durable transaction carrying the root
instance reference of the scope. The journal therefore enforces the same
parent/child and generation rules on replay; Closing is not VM-only
choreography.

### 2.1 Bounded Closing sequence

`closing = { drain_for = <positive duration>; settle_for = <positive
duration> }` carries two explicit budgets and no action/default field. It is
required on every finite owner whose subtree contains a `Process`, and may be
declared on a persistent owner for explicit `stop`.

The first causal close request commits `l:closing` and wins once. For a Graph,
that reference defines one closing subtree and one clipped budget; live
descendants close inner-first. The phases are:

1. close external admission to the current generation and route later arrivals
   to its bounded successor mailbox;
2. drain already admitted work, including internal deliveries and call
   completions, for `drain_for`;
3. seal remaining internal deliveries as residual cursor consumption and
   cancel open calls in canonical `CallRef` order;
4. settle for `settle_for`; an observed completion still publishes its normal
   typed exit and wins close-once;
5. mark every still-open OUTBOUND `abandoned`, publish one empty
   `OutcomeUnknown` on its typed exit port with trigger `l:recover`, then
   discard any late partner response as a non-causal observation;
6. run `on stop` after the last required exit, commit `l:stop` inner-first,
   reclaim state, and make the waiting successor runnable.

`Process` cancellation that confirms normally produces `Exited`; it is never
rewritten as unknown. A Loop wakeup is generation-scoped and dies on close.
Saga compensation debt is journal-scoped and survives it.


## 3. Ordering — what is guaranteed, what is journaled

```
GUARANTEED : within one mailbox, FIFO — arrival order is processing order.
NOT DEFINED: order across mailboxes (not observable : no shared state).
```

**Joins.** `on (a, b)` buffers early arrivals in the owning Operator instance;
the transaction
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
| per-call timeout | a Connector's `timeout` (§6) | `TimeoutExpired` on the contract's typed call-exit port (`exit` for Process) |
| join eviction | a join's `within` / `last` (§7) | `JoinExpired` on the Operator's declared typed expiry output (conventionally `expired`) |
| instance lifetime expiry | `lifetime = { … }` | the instance's `on stop` sequence (parent = **the envelope that armed the deadline** — last admission for `idle`, creator for `fixed`, creating admission for its tumbling box) |
| loop backoff delay | `Loop.backoff` ([loop.md](loop.md) §4) | the re-injected call (trigger `l:next`) |
| saga completion budget | `Saga.timeout` ([effects-and-sagas.md](effects-and-sagas.md)) | a transaction of the `: Saga` node — trigger `l:timeout`, parent = the instance-creating envelope; emits `SagaExpired` on the ordinary typed `Saga.exit` output and the instance enters `rolling_back` |

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
codex.exit ────► typed outcome envelope TimeoutExpired
                parent_id = e42          (the cause — the timed-out request)
                trigger   = l:timeout    (emitted on behalf of the Connector node)
                case      = TimeoutExpired ∈ ProcessExit
```

*(`l:timeout` is general : it fires for ANY declared budget's expiry, with
parent = **the envelope that armed the budget** — the request for a call, the
instance-creating envelope for `Saga.timeout`. One trigger, one rule.)*

No new handler form: the graph reacts by ordinary wiring and union
narrowing (`codex.exit -> triage`, guards on `data is
TimeoutExpired`). The output name and union type come from the resolved
Connector contract; timeout does not create a privileged error channel.

### 6.3 The call-closed rule (anti-ghost)

**A timeout completion closes the call, definitively.** If the response arrives late, the
Connector's correlator discards it and the discard is **journaled** (`late
response, discarded` — visible to observability, invisible to the graph).
One admitted call produces exactly one terminal `exit`: a response completion
or a timeout, never both. Any `out`/`err` frames carry the same call identity,
and none may follow that terminal barrier.

This call lifetime is nested inside, but distinct from, its owning Node
instance's lifetime. `stop` closes that current instance; it is not a
substitute for the `exit` of any call already admitted by that instance.

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
  resident-resource `run_id` ends and its cleanup hook runs) → the `restart`
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

### 6.6 Session supervision (generic H1 fold; network integration H2–H6)

A session is not a long call and its messages are not short calls. It is one
resource scope nested inside the owning Node instance:

```text
session open ── message in/out* ── one typed session exit
```

The journal-derived `SessionRef` correlates the whole scope. Each outbound
message still obeys journal-before-effect, but settling its local write
obligation does not publish a call `exit`. Only clean close, protocol
failure, transport loss, session timeout, or bounded-Closing uncertainty
publishes the sole typed session `exit`; no message may follow it.

Node-instance `stop` remains the outer boundary. M1.6 extends the bounded
Closing fold to open sessions: stop closes admission, drains already admitted
session messages, requests protocol close/cancellation in canonical
`SessionRef` order, settles observations, emits the typed terminal or empty
`OutcomeUnknown`, and only then permits `on stop`. This reuses the phases in
§2.1 and does not let a session terminal replace the Node's `l:stop`.

Handshake timeout and whole-session lifetime are contract/session facts.
Application-message acknowledgement, retry and reconnection remain graph
protocol. A transport write alone never proves that the peer application
processed a message.


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
settle.expired ──► typed outcome envelope JoinExpired
                 parent_id = the evicted envelope   (the cause)
                 trigger   = l:evict
                 case      = JoinExpired ∈ the declared `expired` port type
```

The graph decides by ordinary wiring — compensate, alert, DLQ via `Memory`, or
`-> nothing` if abandonment is genuinely normal (then it is *written*, not
silent).

R6 fixes candidate selection precisely. Correlation never leaves the owning
Operator instance. The arrival that first completes a presence set is its
causal trigger, but the selected tuple and transaction key 34 remain in the
handler's declared port order. For a content predicate, candidate tuples are
visited lexicographically by per-port FIFO position; a false `where` consumes
nothing. `last N` retains at most N candidates on each declared input and
evicts the oldest excess candidate. `within D` arms timer kind 2 for each
routed candidate; a stale timer whose cursor was already consumed is a no-op.
If both bounds apply, the first runnable eviction fact wins through the same
reserve/commit/remove discipline as a firing.

In the executable M1.4 subset a bounded join declares the exact conventional
output `expired: union { JoinExpired {} }`. This name is an ordinary typed
port convention, not a privileged error channel; the requirement keeps v1
expiry routing statically enumerable. `having` remains a named future gate.
A partial join with no runnable tuple is quiescent retained mailbox state, so
`drain` may rest while its dated obligation remains armed.

**The boundary that avoids the false friend** : this rule applies to **join
buffers only** (an envelope that awaited presence and never got it = a lost
fact → the typed `expired` output). **Window/aggregate eviction is normal semantics** — a closing
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


## 9. Windows are lifetime profiles

A window is not a separate runtime mechanism. Key selection answers *which
logical address?*; lifetime answers *when does the instance end?* The two are
orthogonal, so a temporal singleton is a real window within its enclosing
Graph address (App-global only at singleton ancestors), while a keyed owner is
a window per key.

| Family | Shape | One envelope belongs to | Rastok | Status |
|---|---|---|---|---|
| **Session** | elastic, closed by admission silence | 1 instance | `lifetime = { kind = idle; after = duration("2h") }` | executable R4 |
| **Fixed** | age from instance start | 1 instance | `lifetime = { kind = fixed; after = duration("30s") }` | executable R4 |
| **Tumbling** | epoch-aligned boxes | 1 box | `lifetime = { kind = tumbling; every = duration("5m") }` | executable R4 |
| **Sliding** | fixed, overlapping | **N boxes** | no accepted spelling yet | deferred |

The table describes the executable lifetime semantics. Persistent, idle,
fixed, single-pass, and tumbling validate on Graph and Node owners. Every
finite owner containing a `Process` must carry the explicit Closing policy of
§2.1. A Connector's optional per-call `timeout` remains an independent normal-
operation budget rather than an implicit close grace period.

The processing-time tumbling contract has four rules:

1. an envelope belongs to the box open at mailbox admission;
2. boundaries are `epoch + k·every`, never relative to the first envelope;
3. the canonical box start is a runtime-owned address frame, not application
   data fabricated by the author;
4. box expiry follows the same Closing protocol and `on stop` ordering as any
   finite instance.

The box-start frame is part of the logical address. One box address admits
only generation zero in forward execution: a delayed pre-close route remains
in that box, while a post-close route refuses until time selects the next box
address. It never creates generation one for an expired box.

In simulation, civil time is the scenario's `origin` plus the virtual offset
(`origin` defaults to the epoch, simulation-semantics §5.3), so the same
admissions select the same boxes. Event-time and watermarks are an additive
future layer. Sliding remains deferred because overlap makes one admission
select several instances and therefore requires explicit one-to-many routing.


## 10. Trigger vocabulary

Two lifecycle triggers belong to the `l:` registry
([envelope.md](envelope.md) §1.1/§3.1):

| Transaction | `trigger` | `parent_id` (the cause) | Emitter (`owner_path` in the record) |
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
only on empty mailboxes, and byte-identical per-address traces across runs.


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
