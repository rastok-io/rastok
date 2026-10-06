# The Execution Model — mailboxes, ordering, time, and the two planes

> **Status (2026-10-06): normative target, categorical package v20.** Graph scopes own
> lifecycle, not application transactions. Operators and Connectors share the
> transactional execution mechanism; temporal managers own their protocol transactions.
> [R9 qualification](../architecture/rework-r9-qualification.md) records the
> implemented Dispatch, routing, Loop and recovery boundaries.
> The [C4 ledger](../architecture/retry-c4-qualification.md) qualifies native
> Retry and typed measurement retention on those same boundaries.
> Companion to
> [envelope.md](envelope.md) (identity, journal-relative contract) — this doc
> pins the rules envelope.md deferred to the execution model : the
> `emission_index` rule, the mailbox-FIFO ordering contract, the dated-events
> queue, and the supervision vocabulary.
> Reference simulations 1–2 exercise mailbox-per-instance ordering and the
> virtual-clock contract.

The question this doc answers : **when a graph runs, what happens exactly, in
what order — and what can be reproduced ?**


## 1. The unit of work — the Executor transaction, run-to-completion

A transaction belongs to an **Executor** or to a temporal Extension manager.
An Operator transaction consumes an activation and evaluates one checked
Dispatch; a Connector transaction consumes a spatial delivery or typed Driver
activation and applies the Connector's Contract; a manager transaction advances
its journal-owned protocol from a retained command or application/exchange
obligation. Retry has no authored command. In each case the abstract shape is
`f(state, activation) → state', ordered products`.
"Transaction" is meant literally, not as a metaphor — **A**tomic (one commit,
all-or-nothing : the transaction record, delivery-semantics), **C**onsistent
(types + Dispatch/contracts), **I**solated (one instance mailbox or manager
operation, one worker at a time),
**D**urable per the `durability` scope. *(Other traditions call
this unit a Petri* transition firing *or an actor* turn *; Rastok says
transaction because that is what it does.)*

**Naming rule** : bare "transaction" in Rastok docs ALWAYS means the
Executor/manager unit. A Graph has no application transaction. The lifecycle
controller may still commit `l:start`, `l:closing`, and `l:stop`
TRANSACTION records attributed to any Node owner, including a Graph. Those are
controller facts, not Graph code. Wider *graph transaction* and *cascade
transaction* remain reserved and unspecified. Shorthand: `tx`.

The contract is **as-if-run-to-completion (as-if-RTC)** :

- Observable semantics : for one logical Node instance, transactions execute one at a
  time, each *as if* uninterrupted, in arrival order.
- Runtime freedom : underneath, the runtime may suspend on I/O, work-steal,
  move keys across workers, or speculate — anything, as long as the observable
  behavior is identical to the one-at-a-time execution. (The same pact as the
  C compiler's as-if rule and database serializability.)

Within an Operator transaction, the Dispatch selects one branch and its linear
statements run in source order. Connector products follow their closed
contract order. This makes emission identity derivable (§4).

An Operator is pure with respect to the external world, but its evaluation can
still refuse (for example, a dynamic narrowing has no match or encoded data
cannot be materialized). Such a `Fault` is a supervision fact, not a graph
value: the transaction commits neither its input cursor, state nor emissions,
and the VM returns the fault to its caller. The conventional output name
`err` is never selected automatically. When rejection is expected domain
behavior, the Dispatch expresses it explicitly and emits a value on a declared
output — conventionally `err`, giving the simple shape `I -> Result<R,E>`.
Catastrophic host failure such as an allocator abort is outside this typed
contract; the runtime does not pretend every hardware/process failure is
recoverable in-process.

### 1.1 Dispatch and preparation

An Operator handler owns one recursive
`Dispatch = Linear | ClosedMatch | OpenMatch`. A ClosedMatch has a finite
tagged/enum/bool domain, disjoint exhaustive arms, and canonical case order.
An OpenMatch evaluates predicates in source order, chooses the first true arm,
and requires `_`. A match is the final control form of a block or arm; only
pure `val` declarations may precede it.

On one port, `where P` is exact sugar for an OpenMatch with
`P => body; _ => nothing`. False consumes the delivery and commits an empty
transaction. On a join, `where` is deliberately different: it runs while
choosing candidates, and false consumes nothing, advances no cursor, and
commits nothing. The accepted tuple is then consumed before its Dispatch.

Before commit, preparation resolves the selected branch, state delta, ordered
emissions, manager commands, route cases, target instances, and every required
capacity. Driver commands remain inert during preparation. Commit makes the
whole set visible; only then may an effect command reach a Driver.

### 1.2 Connector activations

A world-originated Event, inbound Call, Session open, or inbound Session
message is admitted and journaled as INGRESS before the Connector transaction
which consumes it. A result correlated with an already-open outbound Call or
Session write is not another root: it settles that EGRESS obligation and
enters the bounded Connector activation path with the existing causal
reference. Both paths produce ordinary core-owned Connector transactions.
The Driver never writes the journal or semantic state directly.


## 2. M:N — a mailbox per logical instance, a bounded pool of workers

Not a thread per key (100 000 sessions would kill the OS). One bounded
activation mailbox per logical **Executor** instance, plus the bounded command
inbox of each temporal manager, and a bounded worker pool:

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

A Node without `instance` is a singleton within its enclosing Graph
instance. A keyed Executor has one mailbox per canonical key at each enclosing
address. A Graph has a logical instance and lifecycle but no application
mailbox. A `var` is private to its declaring Operator, so no graph-wide lock
or cross-mailbox mutable state exists.

### 2.1 Instance addressing and lifecycle

The M1 reference machine is the physical one-worker case of the model above.
It does not promise order across addresses, even though one deterministic pump
makes an order reproducible in that implementation.

An edge crossing a keyed owner is decided before its producer commits. The
`instance.key` expression sees the complete immutable entering envelope —
including `meta.sys` — and immutable values from enclosing lexical scopes.
The owner body is entered only after address selection, so its own `val`s
cannot participate. The producer reserves destination capacity and records
the canonical target instance reference `(address, generation)`. Delivery and
recovery consume that recorded fact; neither re-routes by evaluating the
expression after commit. The hierarchical address selects placement; its
journal-derived generation forms the instance reference that indexes mailbox,
derived immutable instance bindings, Executor state, open calls, lifecycle,
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
its first application transaction or contained delivery. An Operator may own
`on start` and `on stop` lifecycle handlers. A Connector's lifecycle comes
from its Contract; a Graph has no handlers. Graph lifecycle facts are
committed by the generic controller. A Graph close requests inner-first
closure of live descendants
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
   cancel open calls in canonical `CallId` order;
4. settle for `settle_for`; an observed completion still publishes its normal
   typed exit and wins close-once;
5. mark every still-open EGRESS `abandoned`, publish one empty
   `OutcomeUnknown` on its typed exit port with trigger `l:recover`, then
   discard any late partner response as a non-causal observation;
6. run an Operator's `on stop`, or the corresponding Connector's Contract-defined
   closure, after the last required exit; then commit `l:stop` inner-first,
   reclaim semantic state, and make the waiting successor runnable. A Graph
   contributes no handler at this phase.

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

- Operator emissions occur in the selected Dispatch leaf's **source order**;
  Connector emissions use their closed Contract order.
- **`emission_index` counts emissions per output port, per transaction, starting
  at 0**, in that text order.

```
on in {
  out = { … }                   → ("out",   0)
  out = { … }                   → ("out",   1)     // second emit, same port
  (audit, alert) = m            → ("audit", 0), ("alert", 0)   // RHS once, two envelopes
  (left, right) = (a, b)        → ("left", 0),  ("right", 0)   // values left-to-right
}
```

Conditional emissions count only when they execute — the index reflects the
*actual* emission sequence of this transaction, which is replay-identical because
the transaction's inputs are (journal-relative) identical.

An ordered multi-emission creates one EnvelopeId per target and commits all
emissions or none. It differs from `source -> { a b }`: that routing set
creates one envelope with several DeliveryRefs, and its target order is
canonical rather than semantic.


## 5. The two planes — logical time and the dated-events queue

The semantic core never reads an operating-system clock. The hosted
**observation/supervision plane** maps one civil-time origin plus monotonic
elapsed time onto an explicit logical timeline; the Lab supplies that timeline
directly. The VM uses it for dated obligations and fixes one instant for every
transaction. A handler may observe that instant through `now()`, in which case
the transaction seals it as an input fact. Everything else the supervision
plane decides enters the graph as a **journaled event**:

```
  ┌──────────────────────────────────────────────────────┐
  │ OBSERVATION / SUPERVISION PLANE     (non-deterministic)│
  │ maps hosted time; watches per-call budgets and         │
  │ liveness. decides : timeout, eviction, expiry.         │
  └───────────────┬───────────────────────────────────────┘
                  │ every decision = a journaled event
                  ▼
  ┌──────────────────────────────────────────────────────┐
  │ DATA PLANE = the graph              (deterministic    │
  │ given the journal — §3)                               │
  └──────────────────────────────────────────────────────┘
```

Concretely, the supervision plane maintains **one queue of dated events**:

| Dated event | Armed by | Delivered as |
|---|---|---|
| timer event | a declared `Timer` Connector | a root timestamp envelope (inbound) |
| per-call timeout | a Connector's `timeout` (§6) | `TimeoutExpired` on the contract's typed call-exit port (`exit` for Process) |
| join eviction | a join's `within` / `last` (§7) | `JoinExpired` on the Operator's declared typed expiry output (conventionally `expired`) |
| instance lifetime expiry | `lifetime = { … }` | the instance's `on stop` sequence (parent = **the envelope that armed the deadline** — last admission for `idle`, creator for `fixed`, creating admission for its tumbling box) |
| Loop re-entry delay | `Loop.delay` ([loop.md](loop.md) §9) | manager consumes a durable Next/Again command, then exactly-once `l:next` re-admits on the remembered entry when due |
| saga completion budget | future Saga decision | not executable in M1; no timeout or output contract is inferred |

In production the queue is advanced by the hosted time source; in simulation,
by the fixture (§8). **The delivery contract — order, causality, identity — is
identical in both modes**; that identity is the foundation of two-mode
equivalence. Replay re-reads the journaled events ; it never re-observes and
never re-runs a race.

Arming/due instants and host commit time remain journal annotations and
observability facts — **never** envelope fields. The narrow exception is an
explicit `now()` observation: the transaction's fixed logical instant is
sealed under canonical key 39 because it may change control flow. It remains
absent from time-insensitive firings (the `created_at` lesson,
envelope.md §8).

Unsealed key 41 retains for exact timer schedules alongside
their accepting records, outside the causal projection. This retention is
atomic and integrity-protected, not best-effort observability. Its ordered
arms preserve the original due instants and tie order; a restart cannot turn
a missing instant into `now + duration`. Recovery restores supported Loop, Retry, Call, join and lifecycle obligations
from the original retained instants.

A resumed sweep preserves the unfinished dated frontier before executing
lifecycle work queued by an expiry. Only an instance's own lifetime is fenced
while its start is pending: a successor cannot expire before its retained
start commits. This is not a global priority for starts. Neither a restored
`l:closing` nor a newly admitted child permits its lifecycle transaction to
overtake another event that the uninterrupted sweep would resolve first at
that instant.


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

*`l:timeout` is general for every currently admitted timeout budget: its
parent is the exact envelope which armed that budget. Saga has no executable
timeout contract in M1; its future decision must either reuse this rule or
allocate a distinct fact explicitly.*

No new handler form: the graph reacts by ordinary wiring and union
narrowing (`codex.exit -> match { TimeoutExpired -> triage; _ -> ... }`).
The output name and union type come from the resolved
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

### 6.4 Resource failure and Node-instance closure

A call-level failure is not automatically a Node-instance failure. The
Process job's `timeout` bounds its finite Call and `on_timeout = kill` is
fixed by that profile; its terminal does not implicitly close the Node.

A resident Process uses a Session, not one nested Call per message. Its
application protocol belongs to Operators, including any request timeout.
Service `timeout`, `on_timeout`, and `restart` are not admitted attributes.
Closing the exact child instance uses `child.instance.stop`; Closing's
retained drain/settlement budgets bound resource shutdown (§6.6).

An observed child `exit` ends that Session. It neither restores process memory
nor stops the Node instance. A new input may open a fresh child in the same
still-open instance, never by timer, never by resending an old write. A closed
generation cannot be revived; successors follow ordinary instance admission.
See D-2026-09-10-2/-3/-4 and the executable D-2026-09-14-1 profile.

### 6.5 Attribute-name collisions — the qualified form

`Saga` may eventually own a timeout. When a Connector's Contract and an Extension, or two
Extensions, declare the same attribute name in one lexical scope, the
**unqualified name is a compile error (ambiguous)** and the **dotted qualified
form disambiguates** — reusing the existing attribute-namespace grammar
(`jwt.cache_ttl`), zero new syntax :

```rastok
graph payment : Saga {
  Saga.timeout = duration("30s")          // saga completion budget
  connector pay : HttpClient {
    timeout = duration("5s")              // Connector call budget
  }
}
```

`Saga` is Graph-only in its deferred direction, so this common pair is
naturally disambiguated by lexical ownership. Same-Node Extension conflicts
remain fail closed; [extensions.md](extensions.md) owns the common rule.

### 6.6 Session supervision (executable through M1.6 H6)

A session is not a long call and its messages are not short calls. It is one
resource scope nested inside the owning Node instance:

```text
session open ── message in/out* ── one typed session exit
```

The journal-derived `SessionId` correlates the whole scope. Each outbound
message still obeys journal-before-effect, but settling its local write
obligation does not publish a call `exit`. Only clean close, protocol
failure, transport loss, session timeout, or bounded-Closing uncertainty
publishes the sole typed session `exit`; no message may follow it.

Node-instance `stop` remains the outer boundary. M1.6 extends the bounded
Closing fold to open sessions: stop closes admission, drains already admitted
session messages, requests protocol close/cancellation in canonical
`SessionId` order, settles observations, emits the typed terminal or empty
`OutcomeUnknown`, and only then permits `on stop`. This reuses the phases in
§2.1 and does not let a session terminal replace the Node's `l:stop`.

Handshake timeout and whole-session lifetime are contract/session facts.
Application-message acknowledgement, retry and reconnection remain graph
protocol. A transport write alone never proves that the peer application
processed a message.

The landed scheduler treats opening and every write as distinct durable
obligations. `Pending` means no protocol ownership transferred and the
ordinary bounded mailbox remains claimed; only the pump retries it. `Started`
transfers exactly once. The Driver then reports correlated opened,
write-settled, inbound-message and terminal facts. The terminal projects as
`l:exit`, after which the fold accepts neither another message nor another
exit. Handshake timeout uses timer kind 9 and is fenced by the logical
`SessionId`, never a socket or task handle.

At the retained Closing settlement deadline, a Session without an observed
terminal receives `abort_session`: an idempotent forced-cleanup handoff for
that exact resource, not a second protocol close and not evidence of success.
The committed close request and sealed deadline authorize it; neither terminal
route backpressure nor a failed terminal commit extends the resource budget.
Its `OutcomeUnknown` still follows the ordinary atomic terminal transaction.
Only after that transaction commits does `release_session` relinquish local
resource state and the registry forget its correlation. Both callbacks are
nonblocking; resource-free Lab Drivers have nothing to clean up. Reconstructing
the journal itself performs no cleanup. Subsequent execution can re-offer an
unsettled expiry using its original reference and deadline, never a successor.
The [resident Process qualification](../architecture/process-service.md) adds
real Unix signal and bounded-cleanup tests. The common callback tests alone
do not claim physical signal or WebSocket abort evidence.

### 6.7 Inbound Call supervision (generic H5 fold)

An accepted inbound Call is direct work of the exact source Connector
instance. Its committed root opens the obligation; a Driver terminal fact can
run only after that instance's lazy `l:start`, and it commits as `l:reply`
with no synthetic mailbox position. Fixed acknowledgement consumes zero
positions. A graph-produced response is first committed and routed to the
exact opening instance, then retained as one real Connector delivery; its
terminal `l:reply` consumes that position atomically. The transaction may
publish ordinary typed outputs and must publish exactly one final `exit`. The
root id remains the Call reference and causal parent, while the source
`instance::Generation` is the generation fence.

The obligation participates in stability, finite capacity and instance
Closing exactly like other admitted work. Closing cannot reach `l:stop` while
an inbound Call remains unresolved. The generic seam deliberately provides no
hidden grace period. H5.1's live `ack` Driver retains no graph response
obligation: after admission it queues the fixed terminal acknowledgement
through the same bounded lane. A proposal that cannot yet commit retains its
transport and byte permits outside the causal fold. H5.2's generic direct
response path similarly leaves a committed response in the ordinary claimed
mailbox while the Driver says `Pending`; no protocol bytes exist yet. Late,
duplicate or unmatched response deliveries commit a consume-only discard.
Final H5 executes `respond`. A positive request deadline is armed with the
accepting root, and the retained response and scheduler timeout compete for
one protocol action and one `l:reply`. A root-mixing return path is rejected
unless its explicit direct-path correlator selects the opening reference
before route commit.


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
  earliest ≤ 30 ?  t=10 Timer event           → deliver → drain (stats emits)
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

In production the same queue runs against the hosted time source; the delivery contract
(§5) is identical — which is exactly what makes a fixture able to *play the
journal's role*.


## 9. Windows are lifetime profiles

A window is not a separate runtime mechanism. Key selection answers *which
logical address?*; lifetime answers *when does the instance end?* The two are
orthogonal, so a temporal singleton is a real window within its enclosing
Graph address (App-global only at singleton ancestors), while a keyed owner is
a window per key.

| Exchange contract | Shape | One envelope belongs to | Rastok | Status |
|---|---|---|---|---|
| **Session** | elastic, closed by admission silence | 1 instance | `lifetime = { kind = idle; after = duration("2h") }` | executable R4 |
| **Fixed** | age from instance start | 1 instance | `lifetime = { kind = fixed; after = duration("30s") }` | executable R4 |
| **Tumbling** | epoch-aligned boxes | 1 box | `lifetime = { kind = tumbling; every = duration("5m") }` | executable R4 |
| **Sliding** | fixed, overlapping | **N boxes** | no accepted spelling yet | deferred |

The table describes the executable lifetime semantics. Persistent, idle,
fixed, single-pass, and tumbling validate on Graph and Node owners. Every
finite owner containing a `Process` must carry the explicit Closing policy of
§2.1. A Connector's optional per-call `timeout` remains an independent normal-
The complete conflict rule lives in [extensions.md](extensions.md).

The processing-time tumbling contract has four rules:

1. an envelope belongs to the box open at mailbox admission;
2. boundaries are `epoch + k·every`, never relative to the first envelope;
3. the canonical box start is a runtime-owned address segment, not application
   data fabricated by the author;
4. box expiry follows the same Closing protocol and `on stop` ordering as any
   finite instance.

The box-start segment is part of the logical address. One box address admits
only generation zero in forward execution: a delayed pre-close route remains
in that box, while a post-close route refuses until time selects the next box
address. It never creates generation one for an expired box.

In simulation, civil time is the scenario's `origin` plus the virtual offset
(`origin` defaults to the epoch, simulation-semantics §5.3), so the same
admissions select the same boxes. Event-time and watermarks are an additive
future layer. Sliding remains deferred because overlap makes one admission
select several instances and therefore requires explicit one-to-many routing.


## 10. Trigger vocabulary

Selected lifecycle triggers owned by this execution slice belong to the `l:` registry
([envelope.md](envelope.md) §1.1/§3.1):

| Transaction | `trigger` | `parent_id` (the cause) | Emitter (`owner_path` in the record) |
|---|---|---|---|
| Call-timeout expiry | `l:timeout` | the timed-out request envelope | the Connector node |
| Join-buffer eviction | `l:evict` | the evicted envelope | the Operator node |
| Inbound Call terminal | `l:reply` | the committed root request | the inbound Connector node |
| Loop re-admission | `l:next` | the consumed Next/Again manager command | the Loop owner |

Manager-command production and consumption are separate append-only facts in
the Loop r2 fold. They do not invent a trigger or spatial port. All
runtime facts follow owner attribution: supervision and managers never emit in
an anonymous out-of-graph name.


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
- [effects-and-sagas.md](effects-and-sagas.md) — operation guarantees and the
  deferred Saga Extension direction.
- [loop.md](loop.md) — `Loop.delay` as a dated event; finite fuel and a
  checked `measure` are data-driven termination witnesses orthogonal to
  wall-clock supervision.
- [extensions.md](extensions.md) — Extension composition and conflicts.
- [grammar.md](grammar.md) — `timeout` / `on_timeout`
  attributes, qualified dotted attributes.
