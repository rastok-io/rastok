# Delivery Semantics — the journal, the transaction, and the boundary

> **Status (2026-10-06): normative target, categorical package v20.**
> Companion to [envelope.md](envelope.md) (identity ; its §7 journal stub is
> filled by §6 below) and [execution-model.md](execution-model.md)
> (Executor/manager transactions, ordering, and the dated-events queue).
> The [C4 ledger](../architecture/retry-c4-qualification.md) qualifies native
> Retry and retained-prefix recovery. Reference simulations 3–4 are historical
> Loop/Saga sketches, not current executable qualification.
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
manager transitions advance their journal-owned protocols (Loop commands or
Retry operation/attempt facts):

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
                   … every clock-valued field (e.g. an EGRESS's creation t)

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
  its producer (INGRESS key 13 / TRANSACTION key 25); recovery never
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
              every open EGRESS's floor        (the §4.4 triage needs it),
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


## 4. The boundary — EGRESS records and the unknown outcome

An EGRESS record describes an effect initiated by the graph. The response to
an inbound Call is instead the `exchange_exit` of a TRANSACTION; it does not
open an outbound Call or create an EGRESS Call record.

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

### 4.2 The EGRESS record — the write-ahead of the call

The journal is the WAL, including the **attempt trace**. Before ③, the
Connector journals an **EGRESS record**; the call has exactly one
closure (the call-closed rule, execution-model §6.3) :

```
   EGRESS record :
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
the EGRESS record retains the canonical digest authority.

D-2026-08-30-3 defines the one M1 credential exception without weakening
journal-before-effect. An authorized secret setting contributes its logical
`SecretName` to the prepared descriptor, while material is fixed by provider
coordinate/version in the immutable run header and never becomes request
bytes in the journal. The Driver may inspect that material during pure
pre-open validation and may transport it only after the EGRESS open wins.
Recovery under another attestation refuses; rotation cannot silently change a
replayed effect.

Offering that open record to an outbound Driver is itself bounded. A
`CallStart::Pending` verdict means no effect started: the record remains open
and the dispatcher offers the same intent again after local capacity returns.
Only `Started` transfers ownership and enters the local exactly-once start
fold. Consequently queue or byte pressure changes pacing but can never be
reported as an external domain fact.

For `std/file@3`, the post-open worker decodes and executes that descriptor
itself; it never re-reads the plan settings. Cooperative cancellation before a
complete read or atomic-rename commit emits nothing from the Driver, leaving
the scheduler's won closure authoritative. A completion observed after another
closure won is handled by the same discarded-response rule as every outbound
protocol. `Dispatcher::resume` starts no existing open call: filesystem recall
is a future recovery-fold decision, not replay behavior.

For `std/http-client@3`, the descriptor additionally seals method,
absolute URL, lossless ordered headers, exact codec body, authentication and
idempotency material. The live Driver may try another declared HTTP version
only while no request byte can have become visible. Once visibility is
possible, an error closes through the ordinary typed result; automatic recall
would be an ambiguous second external operation.

One Call has exactly one closure. Recovery starts no existing open Call again
(§4.4). Repetition under the approved Retry target is a distinct admitted
attempt with a new CallId; it does not give the old Call another closure.

Bounded instance Closing uses the same close-once operation, not a parallel
record state. At its settlement deadline an EGRESS still `open` becomes
`abandoned`; the Connector emits its declared empty `OutcomeUnknown` case with
trigger `l:recover`. A response that won first keeps its ordinary completion.
One arriving later is a discarded observation.

A response closure seals the ordered `(port, data)` drafts that won the race;
they have no envelope identities yet. The following Connector transaction
mints and commits every output envelope together. The final draft is the
call's unique typed `exit` barrier.

The request **digest** is the boundary's integrity seal. Retry reuses the
retained prepared request and resolved settings; it does not re-evaluate
invocation expressions. Recovery checks that retained material against the
digest and original validated plan, refusing disagreement rather than silently
preparing a different operation.

### 4.3 Retry with the byte-same key — and the key-stability rule

> Native Retry is executable in format 20 for finite outbound Calls. Under its
> [operation contract](../architecture/retry-c1-algebra.md), the prepared
> operation, resolved settings and optional partner key are retained once;
> invocation expressions are not re-evaluated between attempts. This does not
> authorize recovery to recall an unresolved Call. The compensation discussion
> below is a future Saga requirement, not a current Connector effect class.

On an unknown partner outcome, repeating an operation with the same business
key is safe only when the partner actually honors the required guarantee.
Neither a key's presence nor deterministic expression evaluation proves that
remote behavior. A Retry uses the retained key unchanged, after an explicit
new admission. Replay itself starts no effect and replays no producer code.

**The key names the business operation, not the attempt** (the
Stripe-semantics rule) :

- **Stable business key = the taught norm** when the partner offers keyed
  deduplication: `idempotency_key = meta.app.payment_id`. Repetition after
  timeout is safe only under that partner guarantee; a partner that retains
  failed outcomes must return the same failure, not silently rerun a declined
  payment.
- **Per-attempt key = an explicit state-carried escape** (`data.operation_id +
  "#" + string(data.attempt)`) for partners that consume the key on failure.
  A Loop Graph must advance that field visibly through `Loop.next`. This is
  a new authored operation, not an attempt of the same Retry operation.
- **A key must not be required to vary per iteration inside a Loop.** Composed
  with the call-closed rule and Loop re-injection, that
  requirement would double-pay on the timeout-with-late-success path:

```
   call #0, key "pay_42#0" ──► the bank executes… slowly
   t=duration("30s") : timeout → call closed → TimeoutExpired → Loop re-injects
   next state attempt=1 → key "pay_42#1"  ← NEW key → the bank sees a NEW operation
   💸💸 double payment. Every rule was fine ; the composition was fatal.
```

  **And the key's converse, for future Saga work**: a compensable step performs
  TWO business operations — the forward call and its inverse. One key
  expression cannot name both : against a key-deduping partner, a
  compensation reusing the forward key is deduplicated INTO the forward call
  (the release never executes ; the fold reports a clean `rolled_back`).
  A future compensation Contract whose forward path declares
  `idempotency_key` must declare a distinct inverse-operation key. M1 has
  no `on compensate` handler; [effects-and-sagas.md](effects-and-sagas.md)
  reserves this requirement for the Saga decision.

  In M1 the partner key is optional, and its presence alone is not a
  repetition guarantee. Retry admission uses terminal visibility and the
  original plan's `idempotent` constant; authored Loop repetition has the
  separate advisory rule in [loop.md §11](loop.md).

### 4.4 Recovery — a triage, not a gamble

At restart, `open` EGRESS records without closure = the set of unknowns :

| Situation | Resolution |
|---|---|
| emission delivered, **no** EGRESS record | no effect started; an explicit continuation may open/start the Call after ordinary admission checks. Read-only replay does not. |
| open, no closure, regardless of idempotence guarantee or key | reconstruct the unresolved Call; never automatically recall it. Resume uses its retained timeout or the explicit bounded-Closing settlement; replay only inspects it. |
| retained timeout wins | close exactly once as Timeout; emit the Contract's TimeoutExpired terminal through the ordinary completion transaction |
| bounded settlement exhausts without a confirmed outcome | emit **OutcomeUnknown** on the Contract terminal (normally exit; trigger **l:recover**, parent = request) and close as abandoned; no invented success or silent recall |
| closure already retained | reconstruct its completion/publication obligation; never restart that effect |

**15 September contact correction.** The previous table proposed blind recall
for idempotence or a present key. That was not the implemented
`Dispatcher::resume` contract and conflicts with D-2026-09-15-1. It is superseded
here, not admitted as a new recovery capability. Under the current Retry policy,
only a selected *closed* terminal can authorize a distinct later attempt;
that attempt is not a second start of the original Call. Durable storage and
Saga triage remain separate gates.

The same empty case is the last resort of explicit bounded Closing when
cancellation produced no confirmed terminal outcome before `settle_for`.
Closing does not recall: it first cancels, allows the
ordinary response race to settle, then abandons only what remains open. The
EGRESS state and `l:recover` envelope are identical facts in both paths, so
replay and compensation need no second unknown-outcome protocol.

**The general principle: recovery rebuilds the dated-events
queue by the same fold that rebuilds state.** Every dated obligation is
derivable from (journal, state): an open call's REMAINING budget (its
creation `t` annotation + `timeout`), a buffered join half's `within` deadline
(its buffering annotation + the window), a live instance's lifetime (last
admission + `idle`, creator + `fixed`, or epoch-aligned tumbling
arithmetic), and a Loop's produced/consumed command fold plus pending delayed
re-entry.
At restart each obligation is rescheduled for its remainder ; obligations
already past **fire immediately, in date order** — a call whose budget
expired during the outage delivers its `TimeoutExpired` late by
circumstance, never silently ; a join half past its window is evicted
(`JoinExpired`) on the spot. Two free bonuses : the **audit trail is
per-attempt** (who called what, when, with which key, how many times —
journal, not reconstruction), and a fixture can write an unresolved `open`
record to test this table — the crash becomes an ordinary simulation case.

### 4.4.1 Inbound Calls — commit the root, then close it once

An inbound Call is not an EGRESS call pointed backwards. The world retains
one proposal while the runtime validates the source endpoint, selects the
exact source instance and all downstream routes, and reserves bounded
capacity. The accepting INGRESS record atomically commits the root, its
`InboundCallId` and source `instance::Generation`; only after that commit may the
Driver observe acceptance or send a fixed application acknowledgement.

The root itself opens the obligation. A terminal Driver observation is judged
against the resolved Connector output table and becomes one ordinary
Connector transaction:

```text
INGRESS(root, call_ref, source_instance)        // opens after bounded admission
        ... graph work ...
TRANSACTION(response producer, route=source)   // commits before response bytes
TRANSACTION(l:reply, cause=root, call_ref)      // outputs*, exactly one exit last
```

The terminal transaction creates no synthetic mailbox cursor and no EGRESS
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
a fourth causal record type. Session opening and inbound messages are INGRESS
facts when initiated by the world. A client opening and every graph-to-world
message are EGRESS intents. Session terminal publication is an ordinary
Connector transaction folded against the same close-once authority. Session
behavior requires the complete subforms specified in [identity.md](identity.md),
including the facts needed to replay it.

Every session has one opaque journal-derived `SessionId`. It correlates
messages and the sole terminal exit while excluding sockets, addresses,
negotiated HTTP version and placement. A direct response may use retained
lineage. When lineage roots mix or a message is stored, an explicit contract
correlator supplies the logical session token; arrival order is never a
correlator.

The exact Contract owns opening authority: WebSocketServer admits peer
openings; WebSocketClient and Process service open locally on input. All
three have duplex roles, so roles alone cannot select the opener. Execution
and recovery use this same Contract fact, never a global effect classification
or an idempotence flag. A server's uncorrelated write never becomes a client opening
or a guessed write to its only live peer.

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

> **Retry disposition:**
> [D-2026-09-14-4](decisions.md#pre-c1-review-amendments--14-september)
> gives intermediate attempt envelopes an explicit retained disposition to
> their Retry manager and a separate consumption fact. They are emitted and
> accounted, never silently dropped or delivered downstream. Final attempts
> still require the ordinary authored destinations. This creates no manager
> port, user command or spatial edge; the
> [refactor contract](../architecture/retry-extension-refactor.md#31-every-attempt-emission-has-a-retained-disposition)
> owns the producer/consumer crash window, qualified by the
> [C4 ledger](../architecture/retry-c4-qualification.md).

Missing disposition is recursive `E-UNWIRED-OUTPUT`, including a nested
Graph boundary and a particular uncovered union case. A deliberate discard is
written `source -> nothing`; it consumes without backpressure. The same case
cannot select `nothing` and a real destination.

A Join's `within` or `last` bound is a producer of `JoinExpired` on `expired`,
even without an authored emission. The source checker and independent core
validator both derive that obligation. An unbounded, otherwise unproduced
`expired` output remains dead. D-2026-09-09-1 corrects the implementation of
this existing rule without changing the expiry fact or its causal encoding.

An output is exempt only when impossibility is re-derived by the core
validator from reachable Dispatch leaves, structure, closed narrowing, the
resolved Contract cardinality, or whole-Graph inaccessibility. Unknown remains
possible. A declared impossible output receives `W-DEAD-OUTPUT`; an
inaccessible Graph receives `W-DEAD-GRAPH`. A Driver which nevertheless
produces an impossible port or case raises `ContractViolation` before
publication.

`RootSink`, `UnwiredPolicy`, and name tests for `err` or `exit` have no
current meaning. Each emission is observed once at its source. Crossing the App
boundary feeds `vm.outputs()` without a duplicate emission or anonymous
observation.

A required input which can never be fed receives `W-DEAD-INPUT`. A missing
return half required by an inbound Call `respond` profile is an error because
it would strand an already-admitted protocol obligation.


## 6. The journal — three record types (fills envelope.md §7)

EGRESS records an effect initiated by the graph, not every message leaving
it. The response to an inbound Call is the `exchange_exit` of a TRANSACTION,
not a separate EGRESS record. Inbound/outbound still describe exchange
direction; INGRESS/TRANSACTION/EGRESS name journal record kinds.

The taxonomy collapses by construction : supervision decisions ARE
transaction records (triggers `l:timeout`, `l:evict`, `l:stop`…) ; explicit
timer firings ARE ingress records (`Timer` roots), while `now()` is an optional
field of the firing transaction itself. Exactly three **causal** types
remain — plus one **derived** type, the SNAPSHOT (§3 : the folded state of
an instance or an open saga ; no new facts, rebuildable, excluded from
traces and replay — an index, not history) :

```
   ① INGRESS record       the world enters
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
   ③ EGRESS record        the graph acts on the world          (§4.2)
        open { key, digest, t } → closed (response XOR timeout) | discarded
```

World-originated Driver facts use INGRESS. A typed response correlated with an
already-open outbound Call or Session write does not create another INGRESS
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


## 7. Saga boundary

Saga remains a named future Extension. M1 does not admit `: Saga`, does not
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
| Across a crash, `durability ≥ local` | full recovery of currently specified cursors, state, Calls, Sessions, deadlines, and Loop r2 fold |
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
- [effects-and-sagas.md](effects-and-sagas.md) — operation guarantees and future
  Saga requirements.
- [loop.md](loop.md) — finite recurrence and the advisory repetition warning.
- [extensions.md](extensions.md) — Extension ownership, Retry's hard admission
  rule and final-only publication.
- [M1.6 HTTP/WebSocket backlog](../architecture/m1.6-http-websocket-backlog.md)
  — the H1 allocation that makes §4.5 executable.
