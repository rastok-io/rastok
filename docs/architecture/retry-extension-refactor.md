# Loop, Retry, Call and Saga — refactor contract

14 September 2026. **Semantic direction approved; implementation not started.**
[D-2026-09-14-4](../spec/decisions.md#d-2026-09-14-4--separate-loop-retry-and-saga-over-one-recurrence-engine)
owns the decision. This document records the repository contact, implementation
order and acceptance evidence still required, including the pre-C1 review
amendments. Current packages remain v18.
The original-plan idempotence authority was settled on 15 September by
[D-2026-09-15-1](../spec/decisions.md#d-2026-09-15-1--keep-idempotence-in-the-original-validated-plan).
The C1 design handoff now consists of the [operation algebra](retry-c1-algebra.md),
[reference traces](retry-c1-traces.md) and [identity inventory](retry-c1-identity.md).
Closing during backoff is settled by
[D-2026-09-15-2](../spec/decisions.md#d-2026-09-15-2--stop-a-waiting-retry-without-publishing-its-held-failure).
These are design artifacts, not executable Retry qualification.

> A Loop advances a state; a Retry repeats an operation. The first proves
> finite iteration; the second proves safe repetition.

Retry is also finite: its attempt bound includes the first Call. Neither
mechanism promises that a remote operation has ceased merely because its
local Call timed out. Reentrancy is a different property, concerning a call
made while a previous invocation remains active, not permission to repeat it.

## 1. Responsibilities

| | Loop | Retry |
|---|---|---|
| Repeated subject | a program-selected state | one retained prepared operation |
| Decision | the body's Next/Again/Done verdict | terminal case selected by a static policy |
| Proof | fuel or a supported decreasing measure | terminal visibility and idempotence; finite attempts |
| Exhaustion seen by the parent | the existing host-terminal `LoopExpired` case | the last failure unchanged |
| Authored surface | `Loop.state`, commands and attributes | attributes only; no state binding, command or port |

`App` remains the public root Extension. A Call remains a Connector exchange,
not another Extension. Saga remains the independent future compensation
protocol. Graphs acquire no application handler, mailbox or transaction.

No global `effect` declaration should prevent an ordinary Call from executing
once. `behavior` remains the determinism axis. The operation's `idempotent`
guarantee is separate: absent/false means no guarantee, not proven impurity.
Its source and fixed/author-declared admission follow the exact Contract;
the checker cannot prove a remote partner's truthfulness. A partner key alone
does not supply the guarantee. Saga may use the same guarantee for forward
and compensation Calls without reclassifying idempotence as compensability.

## 2. Retry source and admission

The following is a future-syntax fragment, not a runnable v18 program:

```rastok
connector pay : HttpClient + Retry {
  idempotent = true
  idempotency_key = meta.app.payment_id
  Retry.attempts = 3
  Retry.delay = {
    kind = exponential
    first = duration("100ms")
    ceiling = duration("2s")
  }
  Retry.on = [ConnectFailed, TimeoutExpired, ServerError]
  // Ordinary HTTP request, codec, behavior and timeout settings follow.
}
```

- `Retry.attempts` is a positive plan constant: 3 means at most three Calls,
  not one initial Call plus three retries.
- `Retry.delay` uses the existing fixed/exponential logical-delay vocabulary,
  with bounded arithmetic, no jitter and no hidden clock.
- `Retry.on` is a canonical static set of the resolved terminal's cases.
  There is no handler, arbitrary expression, payload predicate or alias
  `Retry.cases` in this first profile.
- A selected `before_effect` case may recur without idempotence. A selected
  case with possible/completed external visibility requires the guarantee.
  Compensation does not make an unsafe repetition safe.
- The initial host is a finite outbound Call with bounded completion storage
  and a positive timeout. Event, Session, Process service, inbound Call,
  Operator and whole-Graph retry remain named, unadmitted profiles.

### 2.1 HTTP server failures — approved Contract amendment

Today's `http-client@2` represents HTTP 500 as `Completed { status: 500 }`.
The reviewed target splits a successfully received and decoded response into:

| Terminal | Status | Visibility |
|---|---|---|
| `ServerError { status: int }` | 500–599 | `after_effect` |
| `Completed { status: int }` | every other currently admitted final status | `after_effect` |

Both cases produce the same ordinary response-body `out` before the terminal;
neither denotes a transport fault. `Completed` is not a claim of business
success: 4xx, including 429, remain there. `Retry.on = [ServerError]` selects
all 5xx, not an arbitrary predicate or only status 500. The idempotence
requirement still applies. Body-limit, protocol and decode failures retain
their existing cases; a status does not hide an unusable response body.

This is a versioned HttpClient Contract change, with or without Retry, not a
new Retry exit case or a Driver heuristic. C1 must specify the response-frame
cardinality change from "one iff Completed" to "one iff Completed or ServerError",
including independent validation and forged status/case refusal. Physical
Drivers and the Lab must select the same terminal for the same response.
Current sources, Contract ids and goldens remain untouched until cutover.

The actual retry-on-500 migration is in
[`webhook-to-partner.rkg`](../../examples/webhook-to-partner.rkg).
The [`agent-http-candidate.rkg`](../examples/agent-http-candidate.rkg) `terminal`
Operator currently reports non-2xx as failures; it does not request a retry.
That business classification cannot simply be deleted as retry bookkeeping.

## 3. One operation, several retained attempts

The manager pins the prepared request and the settings that define the
operation: target, method/path, body, ordinary headers and idempotency material
where applicable. Invocation expressions are not re-evaluated to manufacture
a different operation between attempts. The existing secret exception stays:
logical references and immutable binding attestations are retained, never
clear credentials in the journal. Retry does not silently rotate a binding.

Each attempt receives a new Call identity and exactly one closure. All
attempts are correlated with the exact opening admission and owner generation;
business `origin_id` alone cannot distinguish two deliveries of one root.
Within one Retry, changing that pinned request must refuse before another
effect. This is not a new global deduplication registry. The operation's key
is not incremented with the attempt ordinal.

Only the terminating attempt publishes its ordered host outputs and terminal
to the graph. That may be success, an unselected terminal case, or the last
selected failure after the attempt bound. The last failure's type/payload is
not replaced with `RetryExpired`; ordinary envelope identity and causality
still reflect the retained producer. There is no new port.

Every intermediate attempt remains an OUTBOUND fact with its real completion.
The selection and publication decision must be reconstructible and accepted
exactly once. Recovery between settlement and publication must neither lose a
final result nor publish it twice. A late outcome of an already closed attempt
cannot win over its retained closure or settle another attempt.

### 3.1 Every attempt emission has a retained disposition

An intermediate attempt's Connector transaction still creates and journals
its ordered `out`, `err` and terminal envelopes. They are emitted, not delivered
to downstream Nodes. Their disposition is the exact Retry manager, derived
from the validated policy, terminal and remaining attempts; it is neither an
implicit `nothing` nor an omitted route. The producer retains that disposition
atomically with its emissions, after reserving bounded manager receipt capacity.
No manager port, authored command or `PhysicalEdge` is introduced.

The manager's consumption is a separate retained fact, called `AttemptConsumed`
in this algebra: operation, attempt, Call, producing transaction and exact
ordered emission references. It acknowledges the complete attempt batch before
another attempt is admitted. The shared fold rejects a missing producer,
omitted/duplicated emission, wrong run or repeated consumption. C1 owns its
encoding, not a new top-level journal kind allocated here.
Quiescence excludes this manager receipt itself, just as Loop excludes its
pending command; otherwise the consumer would wait on its own consumption.

After a crash between production and consumption the batch remains pending;
neither the Connector transaction nor the settled Call is re-executed. Manager
consumption advances no downstream mailbox cursor. Final attempts instead use
the authored graph dispositions, with no second emission or new EnvelopeId.
Every case can be final when attempts are exhausted, so selecting it in
`Retry.on` never exempts it from explicit output wiring. An unconnected final
output still fails `E-UNWIRED-OUTPUT`.

## 4. One recurrence manager and fold, distinct policy causes

The refactor shares the manager and journal-derived recurrence foundation,
not merely the delay helper. Common obligations are opening, exact admission,
bounded ordinal progression, waiting for a retained due time, quiescence,
capacity admission, generation fencing and closure. There is no separate
Retry ledger or mutable counter authority.

The policy-specific payload remains truthful:

- Loop supplies a state envelope and an authored command reference.
- Retry supplies the pinned operation and the exact closed Call whose
  terminal selected the static policy.

Thus Retry has internal protocol state, but no program-owned state or
`Retry.state`. It cannot produce fictitious `CommandProduced(Again)` facts.
The common fold must represent these two causes explicitly and reject a
policy/cause mismatch. Loop's command production/consumption crash window
remains covered; Retry's settlement/next-admission window is covered without
pretending that an Operator made the decision.

Current `journal/loop_protocol.rs` is not yet that common algebra: `Advance`
requires a `CommandRef` and an envelope state, and the fold validates them.
Historical r2 packets cannot simply be relabelled Retry packets. The versioned
refactor must enumerate the common facts and policy-specific payloads before
allocating or blessing their encoding. Shared facts do not require every
policy to emit every kind of fact: Retry never emits a Loop exhaustion value.

Closing wins over a due retry; nothing migrates into the owner's successor
generation. Quiescence is established from retained obligations, not inferred
from wall-clock silence. Replay/recovery reconstructs the manager without
calling an old Driver effect; idempotence alone is not an execution command.
As settled in D-2026-09-15-2, stopping during backoff consumes any pending
manager batch for Stopped and publishes no new graph output. An already
consumed batch is not consumed again. The original failure remains retained;
neither replay nor Closing turns its manager disposition into a public route.

### 4.1 Delay origin and retained time

`Retry.delay` starts at the retained logical instant of the winning Call
closure, not at request opening, manager consumption, Driver wall time or
restart. For attempt ordinal `i` starting at 1, a selected failure with another
attempt available fixes `due = closed_at + delay(i)` using the existing
saturating delay arithmetic. Delayed recurrence uses timer kind 5 in the common
manager; C1 must preserve old Loop references and distinguish the policy's
references in the versioned algebra, never reinterpret an old Loop wakeup.

The closure instant and exact due time must survive the crash before manager
consumption. Recovery never grants a fresh delay. Admission waits for both the
due time and retained quiescence/capacity; a late consumption can make the
attempt immediately eligible, not move its deadline. No downstream capacity
is held during the delay. Closing wins and a final attempt arms no retry timer.

Current `CallRecord` has no closure timestamp; `Intent.logical_time` describes
the opening context, and `Record::timer_schedule()` returns none for OUTBOUND.
Thus C1 must specify this temporal retention before its identity inventory.
Reuse the existing distinction between mandatory retained time and the causal
seal; do not infer the missing instant from record position or an opening time.

## 5. Composition

Containment is `Loop -> Retry -> Call`, never `Retry -> Loop`. An ordinary
Loop Graph containing a Retry Connector is the initial composition target.
A same-owner `Loop + Retry` needs the same unambiguous nesting and qualification,
independent of Extension spelling order; it is not admitted by manager ordering.
Retry attempts do not consume extra Loop fuel: they belong to one application.

The Call-only staging is confirmed. A future whole-Graph Retry must establish
idempotence for **every Call it can repeat**, including a successful sibling
of the Call that failed; the selected terminal alone is insufficient. This is
a necessary admission rule, not a complete Graph profile. Preserving each
logical operation's request/key, private-state behavior between attempts,
aggregate termination and final publication must be decided and qualified
before that profile opens. It contains no Loop iteration.

An authored Loop `again` that may repeat an externally visible Call without
an idempotence guarantee requires a named warning, not a restored global
effect error. The analysis includes sibling Calls and treats absent/false as
no guarantee; a path proved to have failed entirely `before_effect` does not
warn. The warning identifies the Loop, `again` site and repeated Connector in
canonical order. It does not add retries or certify safety. `W-LOOP-REPEAT` is
the proposed code, to register with its focused warning tests in C2 under the
[diagnostic registration rule](../spec/diagnostics.md#rules); this planning
pass allocates no executable diagnostic. Retry's own unsafe-attempt refusal
remains an error. Intentional state-changing `Loop.next` is not silently
reclassified as Retry by this warning.

In a future Saga, multiple attempts of one operation are not multiple successful
steps. Separate Loop applications can undertake separate operations and thus
incur separate compensation obligations. A compensation is a new operation,
with an identity distinct from the forward operation and its own repetition
guarantee. Neither an unknown outcome nor HTTP `Completed` by itself establishes
business success. Saga composition, step declarations, pivots and compensation
execution remain the separate Saga gate, not work smuggled into Retry.

## 6. Accounting and inspection

Graph publication and accounting visibility are different decisions. Every
attempt's observed usage remains attributable to that attempt and operation,
including failed attempts withheld from downstream ports. The final attempt
must not be counted once from its closure and again from its published output.
Absent usage/prices remain unknown/unpriced; Retry never invents zero cost.

The existing CostReport path must be audited before output filtering lands:
retained raw completions alone are not proof that the current report extracts
their annotated measures. Policy validation and redaction apply equally to
intermediate completions. No new counter, reconstructed fake envelope or
observation stream becomes the accounting authority.

The debugger must distinguish an iteration, an operation and an attempt, and
show all retained attempts without drawing Retry commands or ports. Physical
time and repeated billing are not made deterministic by idempotence. The
ordinary one-model HTTP adapter needs neither Saga nor a false idempotence
claim to make a single invocation after the global effect gate is removed.

## 7. Contact with the repository

| Current authority | Consequence for the refactor |
|---|---|
| [`node::Extension`](../../crates/rastok-core/src/node.rs) and [`plan::ExtensionSpec`](../../crates/rastok-core/src/plan.rs) are closed | add Retry only with complete source, transport and independent validation; no speculative tag |
| [`TerminalVisibility`](../../crates/rastok-core/src/connector/call.rs) already classifies each terminal | retain the Contract facts; separate their consumer from the global effect enum |
| [Compiler check](../../crates/rastok/src/compiler/check.rs) admits only `none`/`idempotent` effect words | remove the global effect gate at cutover, not by calling paid inference idempotent |
| [Plan validation](../../crates/rastok-core/src/plan/validation.rs) and [VM dispatch](../../crates/rastok-core/src/runtime/vm.rs) also use effect as an initiation classifier | rebase Calls on roles/exchange and Sessions on their Contract opening rule; roles alone do not distinguish the two WebSocket profiles |
| [`Intent`](../../crates/rastok-core/src/boundary.rs) retains `EffectClass`; [Call recovery](../../crates/rastok-core/src/runtime/vm/recovery/calls.rs) checks equality | audit plan, causal seal and recording together; do not reinterpret a class tag as a boolean |
| [Loop protocol](../../crates/rastok-core/src/journal/loop_protocol.rs) has Loop state and command-shaped causes | generalize the recurrence algebra; never synthesize a command for Retry |
| [Stdlib Process job](../spec/stdlib.md) publishes a bounded completion at exit | use the existing completion boundary for selection; do not claim unqualified streaming retry |
| [Extension admission](../spec/extensions.md#5-extension-admission-test) requires dedicated orchestration and closed composition | pinned attempts and selective final publication justify Retry as a protocol, not Loop sugar |

The normative clauses to supersede are the global effect requirement and
Loop effect-safety coupling in D-2026-09-05-1, effects-and-sagas, loop §11,
the outbound recovery descriptions, and affected Contract sheets/identity
tables. Node ontology, ordinary routing, user data annotations, Call close-once,
logical time, no-recall replay and Saga's execution deferral stay intact.

### 7.1 Effect is not a role — exact Session migration gate

The current overload is real: network validation requires `Effect::None` for
HttpServer and WebSocketServer; outbound Call admission rejects it through
`EffectlessCallee`. VM peer Session opening and orphan-write handling also
branch on `Effect::None`. Those are protocol-direction decisions, not proofs
that an HTTP response or a WebSocket write has no external consequences.

`roles.has_outbound()` plus `exchange` replaces the Call-direction shortcut,
including the inbound Call response exception. It does **not** select a
Session initiator: WebSocketClient and WebSocketServer both resolve to
`InboundAndOutbound` plus `Session` in `validate_network_connector`.
Process service also has both roles and opens on local input. C1 must derive
and independently validate the opening rule from the exact Contract profile:

| Profile | Opening authority | Input without a selected peer Session |
|---|---|---|
| WebSocketServer | peer admission | existing observable orphan-write discard; never open a client Session |
| WebSocketClient | local input | open or reuse this instance's Session |
| Process service | local input | open or reuse this instance's child; an upstream socket ref cannot select it |

No inference from `idempotent`, port names, `url`/`listen` presence or incidental
manager order is permitted. C1 must make that Contract rule available to the
generic VM before removing the overloaded effect checks. No new public trait
or new user-facing setting is required by this constraint.

### 7.2 Plan-owned idempotence and exact Contract versions

**Option B selected, D-2026-09-15-1.** Remove `Intent.effect` without adding
`Intent.idempotent`. The resolved guarantee belongs to the original validated
Connector plan/Contract. In this first profile it is immutable and
plan-constant, not a per-invocation expression. Every attempt stays bound to
that original authority; a changed source file, environment or newer Contract
cannot revise the guarantee during recovery.

The current [recording](../../crates/rastok/src/recording/mod.rs) already embeds
and validates its plan package. Recovery must reject missing or contradictory
original plan/Contract binding, never infer permission from an old effect word
or missing field. It does not need a duplicate guarantee in each Intent.
Per-invocation guarantees are outside this profile; admitting them later would
require an explicit retained binding, not a silent extension of this rule.

The current diagnostic [recording codec](../../crates/rastok/src/recording/codec.rs)
serializes the effect word; [Call recovery](../../crates/rastok-core/src/runtime/vm/recovery/calls.rs)
checks it against the plan. C1 must inventory removal from recording decode
and recovery alongside the plan-owned guarantee and separately defined causal
projection, without pretending that every in-memory field is already a causal
key. The representation choice is closed; its versioned implementation is
still required and is not a mechanical field rename.

Contract majors are allocated per exact profile, not by a blanket "all @3"
rule: [`connector.rs`](../../crates/rastok-core/src/connector.rs) already gives
Process service `process@3`, distinct from job `process@2`. The HttpClient
status split requires a new major; every other affected Contract must be
inventoried individually. No existing id may acquire another meaning.

## 8. Implementation order and exits

| Slice | Required result | Status |
|---|---|---|
| C0 — contract and contact | approved semantic distinction, output publication, audit and migration inventory | recorded here; exact encoding still unallocated |
| C1 — operation and recurrence algebra | pinned operation, retained attempt disposition/consumption, closure-based deadlines, exact Session opening rule, plan-owned guarantee and Intent field removal, shared fold, identity register and crash traces | design handoff written on 15 September: [algebra](retry-c1-algebra.md), [traces](retry-c1-traces.md), [inventory](retry-c1-identity.md); backoff stop settled, no runtime/format changes; review before C2 |
| C2 — source and validated plan | Retry settings, idempotence, Loop warning, typed HTTP 5xx/cardinality, removal of global effect, full registry-free checks, one declared format cutover | not started |
| C3 — runtime and accounting | qualified Call attempts, final publication, shared scheduling/recovery, exact per-attempt report input | not started |
| C4 — migration and qualification | examples, goldens with reasons, UI, docs, no_std and memory/code-size gates | not started |

C1 must enumerate package/Contract/recording movements and non-movements before
any re-blessing. No old tag receives a new meaning. No claim is made that all
Call-bearing witnesses remain byte-identical when their effect representation
changes; expected differences must be recorded first. Unaffected pure witnesses
retain their identity. No new format number or hypothetical public trait is
allocated by this document.

The acceptance matrix must cover:

1. attempts 1 and 3, unknown/duplicate terminal selections and invalid bounds;
2. before-effect repetition without idempotence; ambiguous repetition refused
   without the guarantee; a key alone not waiving it;
3. frozen requests/settings across attempts and distinct Call identities;
4. intermediate failure/data explicitly disposed to and consumed by the
   manager, retained/accounted without downstream delivery; forged or repeated
   consumption refused; final wiring required and result published once;
5. crash prefixes after settlement, before/after scheduling, before final
   publication, plus late outcomes and Closing at the due instant;
6. two keys and two same-origin admissions with independent Retry runs;
7. a Loop application containing several Retry attempts, with fuel charged
   once for that application; no reverse containment;
8. missing usage, exact decimal costs, secret/personal redaction, no double
   counting from closure plus final publication;
9. CLI, Lab batch/step, diagnostic replay and debugger agreement;
10. workspace checks, three no_std targets, embedded heap/code size measured
    before growth is accepted, and explicitly explained golden migrations.

Pre-C1 review adds focused cases to that matrix: complete 499/500/599 responses
and status/case contradictions; a 5xx with decode failure; retained body usage
on both HTTP response cases; delay unchanged after a crash before consumption;
Loop `again` warning for a non-idempotent sibling and no warning on a proven
before-effect-only path; server orphan writes never opening clients; client
and Process-service local opening preserved; missing or contradictory original
plan/Contract binding refused; changing current source cannot change a retained
operation's guarantee. No per-attempt guarantee field survives in the target
Intent/recording representation.

No implementation, runtime qualification, benchmark call, paid model invocation,
release tag or public push is claimed by this preparatory slice.

### C1 contact additions — 15 September

- Recovery currently re-evaluates invocation settings at each Call open;
  Retry must instead validate later attempts against the pinned operation.
- Loop r2 can retain its transport/identity while both policies share one
  recurrence implementation. Fresh Retry facts must not fake Loop commands.
- Retry closure time and its timer arm must be retained on the winning
  OUTBOUND closure, before a potentially late manager receipt.
- Accounting reads typed transaction emissions, not raw OUTBOUND drafts.
  The HTTP candidate decodes usage downstream; withholding its intermediate
  bodies does not run that Operator. C3 must prove measured attempt evidence
  where available and preserve unknown otherwise, not claim extraction it lacks.
- The older recall table in delivery-semantics §4.4 is historical, not current
  permission to call an open effect again. C1 clarifies this discrepancy.
