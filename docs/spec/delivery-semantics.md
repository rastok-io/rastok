# Delivery Semantics — the journal, the transaction, and the boundary

> **Status (2026-09-05): normative, including the v16 target under
> D-2026-09-05-1.**
> Companion to [envelope.md](envelope.md) (identity ; its §7 journal stub is
> filled by §6 below) and [execution-model.md](execution-model.md)
> (Executor/manager transactions, ordering, and the dated-events queue).
> Reference simulations 3–4 exercise Loop retry × outbound WAL and saga
> compensation × crash recovery.
> D-2026-08-25-1 adds sessions in §4.5. H1 makes their generic causal
> subforms and two-phase Driver decision API executable. H2 publishes the
> concrete HTTP/WebSocket sheets and Lab bindings. H3 selects the private
> bounded hosted-engine seam; H4 executes HttpClient with pre-visible-only
> fallback and generic outbound backpressure. D-2026-08-26-5 lands H5's
> generic inbound Call fold; H5.1 binds ack-mode HTTP/1.1–2 to it with bounded
> admission pressure; H5.2 adds the generic direct-lineage graph-response
> return. Final H5 executes ack/respond over HTTP/1.1–3. H6 executes Session
> I/O over all three WebSocket handshakes; H7 closes static assembly and
> portability evidence.

The question this doc answers : **what survives — a crash, a redelivery, a
slow consumer, a dead process — and what exactly is promised at each
durability scope ?**


## 1. The unit of commit — the transaction record

An [Executor or temporal-manager transaction](execution-model.md) commits
**atomically as one journal record**. Operator transitions use Dispatch;
Connector transitions use their Contracts and typed Driver activations;
manager transitions consume durable Extension commands:

```
   transaction record = CAUSAL CORE + OBSERVATION ANNOTATIONS

   causal core   : node, instance reference (address + generation), trigger (p:/l:),
                   consumed positions   (the cursor advances, §2),
                   join_inputs          (the buffered envelope ids a join
                                         consumed from the owning instance's join buffer — the
                                         provenance DAG's other half),
                   state delta,
                   emissions            [(port, emission_index, id, envelope)]
                   manager commands     (produced / consumed, when present)
                   routed targets       (instance references reserved before
                                         this commit)
                   parent instance      (on a non-root l:start only)
                   lifecycle cause      (when no consumed cursor names it)
   annotations   : committed_at (UTC in production, virtual in simulation —
                   the REGULATORY timestamp, commit-authoritative),
                   … every clock-valued field (e.g. an OUTBOUND's arming t)

   ONE commit covers both. Before it : the transaction never happened.
                          After it  : it happened entirely.

   THE RULE : clock-valued fields are observation annotations ; causal order
   is carried by record sequence and ids, never by clock values. The trace
   FINGERPRINT covers the causal core only (simulation-semantics §6) — which
   makes two-mode comparison a real, checkable operation.
```

Delivery and Driver effects are **post-commit and journal-driven**: emissions
reach target mailboxes and effect commands reach Drivers only after the record
is durable (per scope, §6). The two crash
worlds close cleanly :

- **Crash before commit** → the transaction never existed ; the input is
  redelivered (at-least-once from upstream) ; reprocessing derives **the same
  emission ids** (zero randomness in the recipe) → downstream dedup makes the
  retry invisible.
- **Crash after commit** → the emissions are durable facts ; delivery resumes
  from the journal. A keyed target resumes from the route bytes retained by
  its producer (INBOUND key 25 / TRANSACTION key 38); recovery never
  re-evaluates `instance.key` against old traffic or substitutes a later
  generation.

Net observable guarantee **inside the graph : effectively-once** — the
derived-id recipe is the mechanism (where Kafka needs producer epochs and
sequence numbers, Rastok's identity recipe *is* the idempotence).


## 2. Delivery inside the graph — cursors, not id-sets

Dedup does not remember ids (an unbounded set) ; it remembers **a position**.
The journal orders each node's emissions ; delivery reads them in order ;
therefore delivery is **FIFO per (edge × source instance reference)** — and deduping a
FIFO stream takes one integer :

```
   journal of route (alice) :  [ E1@1, E2@2, E3@3, E4@4 ]
   codex's cursor on the edge : "consumed up to 3"
   redelivery of E2 ?  2 ≤ 3 → skip.   E4 ?  4 > 3 → consume, cursor := 4.
```

- **The cursor commits inside the consumer's transaction record** (§1 :
  "consumed positions") — consumption and its effects are one atomic fact.
  No second mechanism.
- **The redelivered id is an integrity seal, not the dedup key** : at a given
  position, the id must be byte-identical to what the journal knows. A
  mismatch is **never a silent skip** — and it is **not a graph error**
  either (the data plane cannot repair corruption) : delivery on that edge
  **halts**, and an operator alert fires on the observation plane. The same
  channel serves a §4.2 request-digest mismatch on retry.
- Dedup state : **O(edges × live source instance references) integers** for the live
  working set. Each committed consumed position is the exact
  `[edge, source_instance_ref, seq]` tuple; the target instance reference is
  independently retained by the producing record and transaction key.


## 3. Bounded everywhere — RAM, flow, disk

Three accumulations, three answers — nothing can grow without limit :

1. **RAM — the mailbox is a window, not a reservoir.** The durable backlog
   lives in the journal ; the in-memory mailbox is a **bounded prefetch
   window** over it. Full → stop prefetching. RAM cannot overflow, by
   construction.
2. **Flow — bounded lag, pressure to the inbound.** The gap between a
   producer's journal position and its consumer's cursor is **bounded per
   edge and live source instance reference**. At the bound, the producer stops
   being served ; the pressure
   propagates edge by edge back to the inbound Connector, which **translates
   it into its protocol** : HTTP 429/503 or slowed accepts, a polling source
   stops polling, a `Process` stops reading stdin. The world sees a slow or
   limited server — never a dying host.
   Backpressure changes **pacing only, never semantics** — arrival orders
   are journaled as they happen ; the journal-relative contract is
   tempo-insensitive. Window sizes and lag bounds are **runtime
   configuration, not DSL vocabulary**. A future authored rate policy must
   pass the Extension admission test or remain an ordinary deployment knob;
   no `RateLimiter` category is reserved.
3. **Disk — pruned below the LIVING low-water mark.** A
   delivery-cursor-only rule is unsafe because it could destroy records
   still required by live obligations:

   ```
   LWM = min( delivery cursors,
              every OPEN saga's floor          (the oldest record of its scope),
              every open OUTBOUND's floor      (the §4.4 triage needs it),
              every LIVE instance's state floor )
   ```

   A bare floor would pin the whole journal (a persistent instance = never
   prunable), so the runtime MAY write **SNAPSHOT records** — the folded
   state of one Node instance (its private vars, join buffers, cursors) or of an open saga (the
   fold-so-far). A snapshot is **derived, not causal** : it contains no new
   fact, is always rebuildable from the causal records, is **excluded from
   golden traces and replay semantics** (the journal-relative contract reads
   the causal journal only), and its cadence is runtime policy. A floor
   advances to its owner's latest snapshot ; pruning regains its freedom
   behind it. Recovery = latest snapshot + the causal suffix — which is what
   makes the `local` row of §6's table true. Under `none`/`local` pruning is
   aggressive by default ; under `global`, retention beyond the LWM is a
   **policy** (provenance, audit, replay). Retention is **capped at
   two levels** : a global runtime cap and a per-app override that cannot
   exceed it.


## 4. The boundary — outbound records and the unknown outcome

### 4.1 The cursed window

Inside the graph everything is repairable. At the boundary there is an
instant where **the truth exists nowhere** :

```
   ① upstream transaction commits (the intent is journaled)
   ② delivery to the Connector (cursor — §2)
   ③ the Connector calls the world  ────────►  💳
   ④ the response returns           ◄────────
   ⑤ the Connector's completion transaction commits
      (closure journaled; ordered out/err/exit envelopes emitted atomically)

   💥 crash between ③ and ⑤ : was the request received ? executed ?
      Nobody knows (two-generals — undecidable, not a bug).
```

### 4.2 The outbound record — the write-ahead of the call

The journal is the WAL, including the **attempt trace**. Before ③, the
Connector journals an **outbound record**; the call has exactly one
closure (the call-closed rule, execution-model §6.3) :

```
   outbound record :
     open       { call, idempotency_key, request digest, t }     ← BEFORE the call
     closed     (response completion)  XOR  (timeout — the l:timeout event)
                XOR  (abandoned — written when recovery triage or bounded Closing resolves by
                      OutcomeUnknown : the call is closed-as-unknown)
     discarded  (late response after closure — observability-visible, graph-invisible)
```

Request preparation happens after every contract setting for that call has
become a typed value and before the open record. A call-varying setting that
changes the external operation must affect the prepared request seal. In
particular, the `File` Driver's prepared descriptor includes the exact
logical path (and replacement bytes for `mode = replace`); two paths cannot
share a request digest merely because their trigger payloads are equal. The
resolved setting map is an execution carrier, not a new causal record field:
the outbound record retains the canonical digest authority.

D-2026-08-30-3 defines the one M1 credential exception without weakening
journal-before-effect. An authorized secret setting contributes its logical
`SecretRef` to the prepared descriptor, while material is fixed by provider
coordinate/version in the immutable run header and never becomes request
bytes in the journal. The Driver may inspect that material during pure
pre-open validation and may transport it only after the OUTBOUND open wins.
Recovery under another attestation refuses; rotation cannot silently change a
replayed effect.

Offering that open record to an outbound Driver is itself bounded. A
`CallStart::Pending` verdict means no effect started: the record remains open
and the dispatcher offers the same intent again after local capacity returns.
Only `Started` transfers ownership and enters the local exactly-once start
fold. Consequently queue or byte pressure changes pacing but can never be
reported as an external domain fact.

For `rastok/file@1`, the post-open worker decodes and executes that descriptor
itself; it never re-reads the plan settings. Cooperative cancellation before a
complete read or atomic-rename commit emits nothing from the Driver, leaving
the scheduler's won closure authoritative. A completion observed after another
closure won is handled by the same discarded-response rule as every outbound
protocol. `Dispatcher::resume` starts no existing open call: filesystem recall
is a future recovery-fold decision, not replay behavior.

For `rastok/http-client@1`, the descriptor additionally seals method,
absolute URL, lossless ordered headers, exact codec body, authentication and
idempotency material. The live Driver may try another declared HTTP version
only while no request byte can have become visible. Once visibility is
possible, an error closes through the ordinary typed result; automatic recall
would be an ambiguous second external operation.

One call = exactly one closure. The recall rows of §4.4 need no new closure
kind — a recall is *the same call* re-attempted, and its reply (or timeout)
closes it through the normal rule.

Bounded instance Closing uses the same close-once operation, not a parallel
record state. At its settlement deadline an OUTBOUND still `open` becomes
`abandoned`; the Connector emits its declared empty `OutcomeUnknown` case with
trigger `l:recover`. A response that won first keeps its ordinary completion.
One arriving later is a discarded observation.

A response closure seals the ordered `(port, data)` drafts that won the race;
they have no envelope identities yet. The following Connector transaction
mints and commits every output envelope together. The final draft is the
call's unique typed `exit` barrier.

The request **digest** is the boundary's integrity seal : a retry must
re-derive a byte-identical request, or the graph changed under the call —
loud error, never a silent divergence (§2's cousin).

### 4.3 Retry with the byte-same key — and the key-stability rule

On an unknown partner outcome, **retry with the re-derived key** : replaying
the same transaction re-evaluates the same Rastok expression on the same
envelope → **the key is byte-identical for free**. At-least-once against the
world ; the partner's dedup makes it effectively-once when the partner
cooperates.

**The key names the business operation, not the attempt** (the
Stripe-semantics rule) :

- **Stable business key = the taught norm** for `irreversible` :
  `idempotency_key = meta.app.payment_id`. Retry after timeout → partner
  dedups (safe) ; retry after a *processed* failure → the partner replays
  the failure (correct — a declined payment must not silently re-run).
- **Per-attempt key = an explicit state-carried escape** (`data.operation_id +
  "#" + string(data.attempt)`) for partners that consume the key on failure.
  The Loop Graph must advance that field visibly in its `next` state.
- **A key must not be required to vary per iteration inside a Loop.** Composed
  with the call-closed rule and Loop re-injection, that
  requirement would double-pay on the timeout-with-late-success path:

```
   call #0, key "pay_42#0" ──► the bank executes… slowly
   t=duration("30s") : timeout → call closed → TimeoutExpired → Loop re-injects
   next state attempt=1 → key "pay_42#1"  ← NEW key → the bank sees a NEW operation
   💸💸 double payment. Every rule was fine ; the composition was fatal.
```

  **And the key's converse** : a compensable Connector performs
  TWO business operations — the forward call and its inverse. One key
  expression cannot name both : against a key-deduping partner, a
  compensation reusing the forward key is deduplicated INTO the forward call
  (the release never executes ; the fold reports a clean `rolled_back`).
  A future compensation Contract whose forward path declares
  `idempotency_key` must declare a distinct inverse-operation key. V16 has
  no `on compensate` handler; [effects-and-sagas.md](effects-and-sagas.md)
  reserves this requirement for the Saga decision.

  What remains mandatory: an `idempotency_key` on every
  `irreversible`. Loop recurrence is additionally constrained by
  [loop.md §11](loop.md).

### 4.4 Recovery — a triage, not a gamble

At restart, `open` outbound records without closure = the set of unknowns :

| Situation | Resolution |
|---|---|
| emission delivered, **no** outbound record | the call never left → call now. Trivial. |
| open + `effect = idempotent` | recall, blindly (safe by declaration) |
| open + `idempotency_key` present | recall with the **byte-same key** (§4.3) |
| open + `compensable`, **no key** | do NOT guess: emit **`OutcomeUnknown`** on the Connector contract's typed call-exit port (normally `exit`; trigger **`l:recover`**, parent = request) and close the record as `abandoned` — no eternal re-triage, no duplicate case. The graph/saga decides: verify, compensate, or park; a deliberate retry is a NEW call. |

The same empty case is the last resort of explicit bounded Closing when
cancellation produced no confirmed terminal outcome before `settle_for`.
Closing does not run the recovery recall table: it first cancels, allows the
ordinary response race to settle, then abandons only what remains open. The
OUTBOUND state and `l:recover` envelope are identical facts in both paths, so
replay and compensation need no second unknown-outcome protocol.

**The general principle: recovery rebuilds the dated-events
queue by the same fold that rebuilds state.** Every dated obligation is
derivable from (journal, state): an open call's REMAINING budget (its
arming `t` annotation + `timeout`), a buffered join half's `within` deadline
(its buffering annotation + the window), a live instance's lifetime (last
admission + `idle`, creator + `fixed`, or epoch-aligned tumbling
arithmetic), and a Loop's produced/consumed command fold plus pending delayed
re-entry.
At restart each obligation is re-armed for its remainder ; obligations
already past **fire immediately, in date order** — a call whose budget
expired during the outage delivers its `TimeoutExpired` late by
circumstance, never silently ; a join half past its window is evicted
(`JoinExpired`) on the spot. Two free bonuses : the **audit trail is
per-attempt** (who called what, when, with which key, how many times —
journal, not reconstruction), and a fixture can write an unresolved `open`
record to test this table — the crash becomes an ordinary simulation case.

### 4.4.1 Inbound Calls — commit the root, then close it once

An inbound Call is not an OUTBOUND call pointed backwards. The world retains
one proposal while the runtime validates the source endpoint, selects the
exact source instance and all downstream routes, and reserves bounded
capacity. The accepting INBOUND record atomically commits the root, its
`InboundCallRef` and source `InstanceRef`; only after that commit may the
Driver observe acceptance or send a fixed application acknowledgement.

The root itself opens the obligation. A terminal Driver observation is judged
against the resolved Connector output table and becomes one ordinary
Connector transaction:

```text
INBOUND(root, call_ref, source_instance)        // opens after bounded admission
        ... graph work ...
TRANSACTION(response producer, route=source)   // commits before response bytes
TRANSACTION(l:reply, cause=root, call_ref)      // outputs*, exactly one exit last
```

The terminal transaction creates no synthetic mailbox cursor and no OUTBOUND
record. Fixed acknowledgement consumes zero cursors. A graph response consumes
exactly the real delivery cursor already sealed by its producer, atomically
with Call closure. Direct lineage (`response.origin_id == root.id`) selects
the exact opening source instance during route preparation; transformed
response data never re-keys the return. Only after the producer commit may
the Driver receive that envelope. A bounded `Pending` result writes no bytes
and leaves the same delivery retained behind the ordinary claimed mailbox;
`Started` transfers the attempt and requires one later terminal observation.

The journal requires owner and generation to equal the opening source,
refuses `l:reply` without the open reference, and closes the reference once.
Crossed Driver facts are non-causal. A response that is late, duplicate or
has no open direct lineage is instead consumed by an ordinary empty
transaction: the discard is replay-visible and cannot write protocol bytes or
create a second exit. The generic fold counts the Call and any retained
response as open work for instance Closing. Final H5 admits `ack` and
`respond` over HTTP/1.1–3. A positive request deadline and a retained response
share one close-once race; explicit direct-path correlation is mandatory when
a join mixes roots.

### 4.5 Sessions — one resource lifecycle, many message effects

A call and a session share journal-before-effect, but not cardinality:

```text
call        open call ── request ── outputs* ── one exit

session     open session
              ├── inbound message*   (bounded proposal, then commit/decision)
              ├── outbound write*    (intent before network bytes)
              └── one session exit   (no message after it)
```

M1.6 adds `ConnectorExchange = Event | Call | Session`; it does not add
a fourth causal record type. Session opening and inbound messages are INBOUND
facts when initiated by the world. A client opening and every graph-to-world
message are OUTBOUND intents. Session terminal publication is an ordinary
Connector transaction folded against the same close-once authority. The exact
v11 subforms and keys landed together at H1, so no intermediate package can claim
session behavior without carrying the facts needed to replay it.

Every session has one opaque journal-derived `SessionRef`. It correlates
messages and the sole terminal exit while excluding sockets, addresses,
negotiated HTTP version and placement. A direct response may use retained
lineage. When lineage roots mix or a message is stored, an explicit contract
correlator supplies the logical session token; arrival order is never a
correlator.

An outbound message has a durable write obligation but **not** a graph-visible
call exit. Local write settlement means only that the Driver accepted
the bytes under the session protocol; it does not claim that the peer
application processed them. If recovery cannot prove whether externally
visible bytes were accepted, Rastok does not replay them blindly: the session
closes through its typed unknown terminal and application-level
acknowledgement/retry may create a new session.

Inbound is explicitly two-phase. The Driver proposes an open or message while
retaining it in bounded protocol state and withholding any application-level
acknowledgement. The runtime then:

1. commits the causal fact and tells the Driver **accepted**;
2. leaves it **pending** when bounded admission is full; or
3. returns a contract **rejection** without minting an envelope.

Only the first or third result may advance a peer-visible application
acknowledgement. For HTTP this governs 202/response timing; for WebSocket it
governs successful upgrade and any acknowledgement defined by the application
protocol. RFC 6455 has no per-message application acknowledgement. TCP ACKs
and HTTP/2-3 receive-window updates are pacing, not processing claims; they may
advance within the Driver's finite intake but cannot create an unbounded
shadow queue. A pending proposal remains in that bounded protocol state until
the runtime decides it.

H6 makes this fold executable for both WebSocket contracts. Session opening,
each durable write and the terminal are offered independently through the
bounded Driver pump. A full Driver returns `Pending`; the already-journaled
obligation remains claimed by the ordinary Worker mailbox and is retried only
by the runtime pump. The terminal is one `l:exit` Connector transaction;
subsequent Driver observations and graph writes are silent/orphaned rather
than creating a second terminal. HTTP/1.1 Upgrade, RFC 8441 and RFC 9220 differ
only below this fold.


## 5. Explicit dispositions and productibility

Outputs acquire no runtime meaning from their name. Each possibly productible
output case has at least one explicit disposition in the validated plan:

- a physical destination;
- the current Graph boundary output, including an App public termination;
- a temporal-manager command;
- `nothing`.

Missing disposition is recursive `E-UNWIRED-OUTPUT`, including a nested
Graph boundary and a particular uncovered union case. A deliberate discard is
written `source -> nothing`; it consumes without backpressure. The same case
cannot select `nothing` and a real destination.

An output is exempt only when impossibility is re-derived by the core
validator from reachable Dispatch leaves, structure, closed narrowing, the
resolved Contract cardinality, or whole-Graph inaccessibility. Unknown remains
possible. A declared impossible output receives `W-DEAD-OUTPUT`; an
inaccessible Graph receives `W-DEAD-GRAPH`. A Driver which nevertheless
produces an impossible port or case raises `ContractViolation` before
publication.

`RootSink`, `UnwiredPolicy`, and name tests for `err` or `exit` have no
v16 meaning. Each emission is observed once at its source. Crossing the App
boundary feeds `vm.outputs()` without a duplicate emission or anonymous
observation.

A required input which can never be fed receives `W-DEAD-INPUT`. A missing
return half required by an inbound Call `respond` profile is an error because
it would strand an already-admitted protocol obligation.


## 6. The journal — three record types (fills envelope.md §7)

The taxonomy collapses by construction : supervision decisions ARE
transaction records (triggers `l:timeout`, `l:evict`, `l:stop`…) ; explicit
timer firings ARE inbound records (`Timer` roots), while `now()` is an optional
field of the firing transaction itself. Exactly three **causal** types
remain — plus one **derived** type, the SNAPSHOT (§3 : the folded state of
an instance or an open saga ; no new facts, rebuildable, excluded from
traces and replay — an index, not history) :

```
   ① INBOUND record       the world enters
        { connector, inbound_seq, envelope, ingress_port, committed_at }
        ingress_port = the graph input entered (default `in`, omitted) ;
        inbound_seq committed ATOMICALLY with the accept ;
        ordering rule : COMMIT-THEN-ACK (the record is durable before the
        runtime returns accepted and any application/protocol acknowledgement
        defined by that Connector leaves).
        Backpressure leaves the Driver proposal pending; rejection mints no
        envelope. A crash between commit and acknowledgement → the
        sender may resend → transport-level redelivery yields DISTINCT
        roots unless the Connector's protocol correlates resends —
        inbound dedup is a per-protocol duty, declared in the Connector
        contract (stdlib.md, case 7). Stated, never glossed.
   ② TRANSACTION record   everything inside the graph          (§1)
   ③ OUTBOUND record        the graph acts on the world          (§4.2)
        open { key, digest, t } → closed (response XOR timeout) | discarded
```

World-originated Driver facts use INBOUND. A typed response correlated with an
already-open outbound Call or Session write does not create another INBOUND
root; it settles that existing obligation and supplies a Connector
transaction. Loop `CommandProduced` and `CommandConsumed` are append-only
fields/subforms of TRANSACTION records, not fourth and fifth record types.

- **Envelopes are stored in the canonical storage form** (envelope.md §5) :
  `meta` Avro, `data` tagged-bytes **lazy** — no re-encoding, no second
  format. Immutability pays again : parent and child sharing the same `data`
  bytes are **stored once, referenced** — a 10 MB passthrough does not cost
  10 MB per hop.
- **The write contract, one sentence** : *a record is durable — in the sense
  of its scope — before its emissions are delivered.* (Group commit and
  fsync batching remain runtime freedoms : the as-if of durability.)

### The scope table — what each `durability` honestly guarantees

| | `none` | `local` | `global` |
|---|---|---|---|
| Medium | bounded RAM | runtime-owned local journal | replicated backend selected by the deployment |
| Crash-restart, same host | ❌ lost — **at-most-once across the crash : loss, never doubling** (no journal → no retry → no double-execution) | ✅ full recovery : transactions, cursors, outbound triage (§4.4) | ✅ idem |
| Host loss | ❌ | ❌ | design intent: another host resumes under the clustered contract |
| Retention | RAM cap | local cap + per-App override | pruning below the low-water mark + retention **policy** (audit, replay) |
| Sagas | future gate (§7) | future gate | future gate |
| `irreversible`/`compensable` connectors | ⚠️ compile warning : guarantees are process-lifetime | ✅ | ✅ |


## 7. Saga boundary in v16

Saga remains a named future Extension. V16 does not admit `: Saga`, does not
define `Saga.timeout`, and has no `on compensate` trigger. In particular,
this document must not manufacture an executable rollback fold while the
command, acknowledgement, reverse order, terminal result, and recovery
protocol are unsettled.

The retained direction is narrow:

- Saga state will be a fold of the existing causal journal, not a fourth
  record type or manager-owned mutable database;
- `effect = compensable` will contribute a versioned Contract compensation
  contract, not DSL code inside a Connector;
- `Saga.compensate` will expose a closed union of compensable steps from a
  Saga Graph;
- every inverse effect will have a distinct stable operation identity;
- local-or-stronger durability is expected, but its exact admission rule
  belongs to the independent Saga decision.

[effects-and-sagas.md](effects-and-sagas.md) owns the complete list of
unresolved gates. Until they are decided, every Saga source form fails closed.


## 8. The guarantees, summarized

| Where | Guarantee |
|---|---|
| Inside the graph | **effectively-once observable** (at-least-once + derived-id dedup by cursor) |
| At the boundary | at-least-once + byte-same idempotency key → effectively-once when the partner dedups ; `OutcomeUnknown` (loud) when nothing safer exists |
| Across a crash, `durability ≥ local` | full recovery of currently specified cursors, state, Calls, Sessions, timers, and Loop r2 fold |
| Across a crash, `durability = none` | **at-most-once: loss, never doubling** |
| Ordering | FIFO per (edge × source instance reference); cross-edge = journal-relative (envelope.md §2.0) |


## 9. Trigger vocabulary

| Transaction | `trigger` | `parent_id` (the cause) | Emitter (`owner_path` in the record) |
|---|---|---|---|
| Recovery resolution of an unknown outbound (no key) | `l:recover` | the request envelope whose outcome is unknown | the Connector node |
| Inbound Call terminal | `l:reply` | the committed root request | the inbound Connector node |

These triggers belong to the `l:` registry
([envelope.md](envelope.md) §1.1/§3.1).


## 10. Non-goals of this doc (deferred, with homes)

- **Multi-host delivery / shard handoff** — deferred (the `inbound_seq` replica
  property is already reserved in envelope.md §2).
- **Prod-incident replay tooling** — deferred (the journal-relative contract
  already reserves it).
- **Group-commit / fsync tuning, window & lag bound defaults** — runtime
  configuration, implementation-validation territory.
- **DLQ Connector catalog** — `Memory` suffices for reference examples;
  every routing to it is explicit.
- **Cross-app / cross-graph transactions** — reserved words only
  (execution-model §1 naming rule); nothing wider than an Executor transaction
  exists in the current contract.


## 11. Conformance scenarios

The following reference simulations must stay consistent with this
contract:

- **Reference simulation 3** (Loop retry × outbound WAL : the double-pay trap
  of §4.3 exercised both ways — stable key vs per-attempt key ; a crash
  between call and closure, resolved by §4.4) ;
- **Future Saga simulation** remains gated until the independent Saga
  decision; no current fixture may claim compensation recovery.


## 12. Related docs

- [envelope.md](envelope.md) — identity recipe ; §7 (journal) now points
  here ; the `l:` registry.
- [execution-model.md](execution-model.md) — the Executor transaction (§1
  naming rule incl. reserved *graph/cascade transaction*), mailboxes,
  ordering, dated events, `timeout`/`on_timeout`, `JoinExpired`.
- [effects-and-sagas.md](effects-and-sagas.md) — effect classes and future
  Saga requirements.
- [loop.md](loop.md) — finite recurrence and repeat-safety rules.
- [extensions.md](extensions.md) — Extension ownership and validation.
- [M1.6 HTTP/WebSocket backlog](../architecture/m1.6-http-websocket-backlog.md)
  — the H1 allocation that makes §4.5 executable.
