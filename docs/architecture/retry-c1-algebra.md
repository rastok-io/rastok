# C1 — retained operations and recurrence

15 September 2026. **Design checkpoint, not executable support.** This derives
the implementation algebra from
[D-2026-09-14-4](../spec/decisions.md#d-2026-09-14-4--separate-loop-retry-and-saga-over-one-recurrence-engine)
and [D-2026-09-15-1](../spec/decisions.md#d-2026-09-15-1--keep-idempotence-in-the-original-validated-plan).
The [C0 contract](retry-extension-refactor.md) remains the scope authority.
The [reference traces](retry-c1-traces.md) and
[identity inventory](retry-c1-identity.md) accompany this checkpoint.
No Rust, package, recording or golden changes in C1.

The C1 review follow-up makes the Loop warning an explicit C2 gate, clarifies
one pending manager batch versus cumulative journal retention, and makes
closure-time retention uniform for outbound Calls under
[D-2026-09-15-3](../spec/decisions.md#d-2026-09-15-3--retain-outbound-call-closure-time-independently-of-retry).

Yoann settled closure during backoff on 15 September: stop the Retry without
publishing a held failure (§8), recorded in
[D-2026-09-15-2](../spec/decisions.md#d-2026-09-15-2--stop-a-waiting-retry-without-publishing-its-held-failure).
This is the C1 design handoff; executable validation and qualification remain
C2–C4 gates, not claims made by these hand-written traces.

## 1. Invariants and unchanged scope

A Loop advances a state; a Retry repeats one prepared operation. The common
recurrence machinery must retain the distinction between an authored command
and a selected terminal outcome. Neither adds a spatial back-edge.

The first Retry host is a finite outbound Call: Process job, File read/replace,
or HttpClient, with a positive timeout and bounded complete output capture.
There is no Retry on an Operator, a Graph, a Session, an Event, an inbound
Call or Process service. A Loop Graph may contain a Retry Connector;
`Loop + Retry` on the same owner remains unadmitted pending its own qualification.

`Loop.state`, `Loop.measure`, entry/terminal resolution, fuel, commands and
host-owned `LoopExpired` remain. Both currently accepted Loop delay forms
remain: restricting Loop to fixed cadence was discussed, not adopted.
`in.data` is not added as an expression alias. Saga remains a future gate.

The guarantees to preserve are:

1. each attempt has one distinct Call and at most one winning closure;
2. every attempt uses the same prepared operation, not re-evaluated arguments;
3. every emitted envelope has a committed disposition;
4. intermediate attempt emissions go to the manager, never downstream;
5. the final attempt is emitted/routed once, with no replacement identities;
6. pending work, time and consumption survive solely through the journal;
7. recovery does not start an old effect; a later attempt requires a new admission;
8. one instance generation cannot give its work to its successor;
9. safety follows the original validated plan/Contract, never a current source file;
10. graph publication never decides whether retained accounting evidence exists.

## 2. The operation and its attempts

An operation is identified by the exact opening delivery and target instance,
not the business origin, partner key, request digest alone or Worker slot.
Two fan-out deliveries of one root into the same instance open distinct operations.
Their operation references, attempted Calls and consumption remain separate.

At opening, pure preparation resolves and retains:

- the original request envelope and exact entry delivery/cursor;
- owner, instance generation and enclosing Loop contexts;
- the prepared request bytes/digest, resolved operation settings and optional key;
- the exact original Connector plan/Contract binding;
- the Retry policy from that plan: total attempts, selected cases and delay.

The plan remains the sole authority for idempotence. There is no guarantee
boolean in an Intent, attempt fact or operation-opening payload. Immutable
request/setting material may be shared physically; the algebra does not demand
a fresh full copy for every reference to it. Secrets retain references and
binding attestations, not credential bytes; recording privacy gates stay closed.

Plan/boot/instance evaluation sites keep their existing meanings. Invocation
settings and request preparation run for the opening operation. They do not
run again to prepare later attempts, including during recovery. A pure
preparation refusal does not manufacture a Call or a retryable terminal.

`attempt` is one-based, `1..=Retry.attempts`; the bound includes the first.
Every attempt gets its own admitted request envelope/Call correlation. The
first uses the opening delivery; following admissions mint a temporal envelope
with the original DATA and travelling metadata, the same `origin_id`, and the
preceding terminal envelope as causal parent. The system id changes. That
change never re-evaluates an invocation setting or changes the pinned partner key.

The Call timeout includes its existing admitted queue time. Opening a manager
must not reset the first request's already-armed budget. Each later admission
arms its own Call timeout; no Call budget runs during the backoff itself.

## 3. Shared mechanism, policy-specific causes

The common fold owns exact generation, admitted ordinal/bound, pending receipt,
due time, capacity admission and closure. Policy data is a closed alternative:

| Policy | Retained payload | Cause which permits recurrence |
|---|---|---|
| Loop | current state envelope and existing command protocol | one produced/consumed Next or Again |
| Retry | original prepared operation and current attempt | one selected winning Call terminal and consumed attempt batch |

There is one recurrence admission budget and one dated scheduler, not one set
of limits per Extension. A waiting operation still occupies its admission slot.
Pending completion bytes remain charged to the existing bounded Call/manager
storage until their receipt or final routing commits. Moving work between
queues cannot create capacity. No destination mailbox is reserved during a delay.

Sharing this kernel does not require rewriting Loop r2 facts into Retry facts.
The proposed transport keeps Loop's existing subform and appends a Retry
subform, both validated by the same recurrence implementation (§5 and the
identity inventory). There is no second journal or synthetic Loop command.
The source-specific validators still check their own proof obligations.

### 3.1 Pending capacity is not journal retention

An operation can have **at most one unconsumed attempt batch**. The next
attempt is admitted only after Consumed/Advance of its predecessor. The
manager therefore needs one pending receipt per operation, not an array of
`Retry.attempts - 1` pending batches. Many live operations still add their
independently bounded pending receipts to the common admission budget.

The journal, in contrast, retains every intermediate batch after consumption:
up to `attempts - 1`, plus the final attempt if one publishes. Consumption
releases pending-manager ownership/capacity, not journal evidence. With M1's
in-memory journal, that history still occupies RAM. Referencing the committed
batch avoids a second retained copy; it does not make the journal free.

For Process job, `max_output` bounds **aggregate raw stdout + stderr**, not
decoded heap size. The bound also does not include all envelope/reference,
terminal, allocation and recording overhead. C3 must measure both pending
capacity and cumulative retention with many small frames as well as large
frames, both channels, attempts 1/3/a larger admitted bound and concurrent
operations. It must include completion drafts, decoded envelopes, consumption
references and recovery peaks. No arithmetic claim that the peak is simply
`attempts * max_output` substitutes for that measurement.

## 4. Exact host rules and repetition safety

An ordinary Call may execute once without a Saga classification. Retry uses:

```text
selected = terminal_case belongs to Retry.on
safe = terminal_visibility is BeforeEffect OR original_plan.idempotent
repeat = selected AND attempt < attempts AND owner permits recurrence
```

Disposition is selected at the winning closure prefix; actual admission also
rechecks that the owner still permits recurrence. Closing in between cancels
the wait (§8), not the already selected disposition.

Every selected case must pass the Contract safety rule
at check and independent plan validation. A later runtime verdict cannot waive
it. A key alone never establishes `safe`. `behavior` is unrelated. Existing
terminal visibility classifications remain except for the explicit HTTP addition.

Exact guarantee resolution for the admitted Call profiles:

| Profile | Original plan guarantee |
|---|---|
| Process job | author plan-constant boolean; omission is no guarantee |
| HttpClient | author plan-constant boolean; omission is no guarantee |
| File read/replace | fixed true by the same-request Contract; contradictory override refuses |

The File rule preserves its currently fixed idempotent contract; it does not
promise the same file contents will be observed twice. The generic boolean
does not introduce Retry support on non-Call profiles.

The HTTP Contract distinguishes usable decoded 500–599 responses as
`ServerError { status }`; all other admitted final statuses remain
`Completed { status }`. Both are `AfterEffect`, both produce exactly one `out`
before `exit`. 499/500/599/600 are classification boundary tests, conditional
on each status already being admitted by the underlying HTTP binding. Decode,
protocol and body-limit failures retain their distinct cases. `Completed`
does not imply business success; 4xx/429 are not automatically retried.

The internal frame rule becomes an exact canonical set of terminal cases
rather than one case string: HttpClient `{Completed, ServerError}`, File read
`{Completed}`, File replace never, Process job zero-or-more bounded frames.
The Contract, Driver, Lab and independent validator must agree; an invented
status/case or frame-count combination refuses before graph publication.

Removing `effect` also removes its accidental protocol-direction role:

| Profile | Opening authority | Request without an existing resource |
|---|---|---|
| outbound Call | local input, from exact roles/exchange | open a new Call |
| HttpServer | peer admission | never manufacture an outbound Call |
| WebSocketServer | peer admission | preserve the existing orphan-write disposition; never open a client |
| WebSocketClient | local input | open/reuse this instance's Session |
| Process service | local input | open/reuse this instance's process |

Both WebSocket Contracts have inbound/outbound roles and Session exchange.
Their opening rule must therefore come from exact Contract identity, not
roles alone, idempotence, port names or the presence of `url`/`listen`.
This is a pure Contract fact, not another public trait or DSL setting.

### 4.1 Required C2 Loop warning

`W-LOOP-REPEAT` remains required by C0 §5 and D-2026-09-14-4. Its omission
from the first C1 checker checklist did not reject it. C2 must register it in
the diagnostic catalog **with its implementation and focused tests**, before
closing the checker slice; this document does not claim a v18 diagnostic.

Warn when an authored `Loop.again` path may repeat an externally visible Call
whose original plan has no idempotence guarantee (absent or false). Include
successful sibling Calls repeated by that application, not only the failed
Call that caused Again. A path proved entirely BeforeEffect does not warn.
An unproved absence of effects is not a proof of BeforeEffect.

Identify the Loop owner, Again site and repeated Connector, in canonical
order. The warning is advisory, does not reject the Loop and changes neither
plan/hash nor journal. Do not reclassify an intentional `Loop.next` as Retry.
Retry's own unsafe selected-terminal rule remains a hard refusal, independent
of this warning. Positive/negative fixtures and declaration-order stability
are listed in the reference traces and C2 inventory checklist.

## 5. Retained transitions and atomic boundaries

The names below are algebra terms, not new DSL commands or a fourth record kind.
An operation reference plus attempt ordinal selects at most one Call/batch.
A batch is the ordered emission list of that Call's Connector transaction,
including the terminal. It needs no independent random or sequence-generated id.

| Transition | Carrier and precondition | Atomic retained result |
|---|---|---|
| OperationOpened | owner-manager TRANSACTION; fresh opening delivery, bounded slot available | exact opening, instance, enclosing contexts and pinned operation; no effect |
| AttemptAdmitted(1) | manager TRANSACTION; operation opened, owner live | authorize the original request delivery without consuming it twice |
| CallOpened(i) | ordinary OUTBOUND open; admitted attempt | Intent plus operation/attempt link; only then offer to Driver |
| CallClosed(i) | ordinary winning OUTBOUND closure | exact outcome, logical closure time and any original retry timer arm |
| AttemptHeld(i) | Connector TRANSACTION after closure, more attempts selected | consume that request cursor; emit the complete batch; retain manager disposition; no graph route |
| AttemptConsumed(i), Advance(i+1) | one manager TRANSACTION; held batch and quiescence | consume exact batch once; retain next ordinal and original deadline |
| AttemptAdmitted(i+1) | manager TRANSACTION; due, quiescent, owner live, capacity proved | fresh temporal request admission from pinned operation; ordinary Call timeout arm |
| AttemptPublished(i), Close | one Connector TRANSACTION after a final closure | consume request cursor; emit and reserve all authored dispositions; close operation |

OperationOpened and initial admission may share one atomic manager transaction;
their ordering is fixed when combined. Neither consumes the opening cursor:
the first Connector completion owns that consumption. Subsequent request
admissions have distinct internal temporal cursors. These are not authored
spatial edges and cannot be reached by ordinary wiring.

`AttemptHeld` references the exact Call and ordered envelope ids in the same
transaction's emissions. No omission, extra envelope or cross-operation batch
is valid. Its manager disposition is committed with those emissions and
reserves bounded receipt capacity before commit. Empty `routes` is not by
itself evidence of that disposition. Final attempts instead use all ordinary
authored destinations, including `nothing`; `Retry.on` does not relax
`E-UNWIRED-OUTPUT`, since every selected case can be final at the bound.

`AttemptConsumed` is separate from production, but consumption and Advance
commit together. Repeated consumption refuses. It neither advances a
downstream mailbox cursor nor creates another envelope. Final publication and
operation Close also commit together: there is no gap needing a second
publication transaction or new EnvelopeId. A failed capacity reservation
commits neither the completion transaction nor its Close.

The manager waits only for obligations of this operation/attempt. Its own
pending batch and retry wakeup do not count against quiescence. A containing
Loop, however, sees the entire Retry operation as outstanding until its
publication/closure; otherwise the outer Loop could finish during backoff.
Other operations in the same keyed instance do not block this quiescence.
Ordinary instance serialization and route ordering still apply.

## 6. Closure-based time, including a late manager

For failed attempt `i`, fixed delay `d` or exponential delay
`min(first * 2^(i-1), ceiling)` gives:

```text
due = saturating_add(winning_closed_at, delay(i))
```

Compute with bounded/saturating arithmetic before multiplication can overflow.
Without a configured delay, no retry timer is armed; recurrence can become
eligible after consumption/quiescence. Neither case promises immediate execution.

The clock input is the VM/Lab logical instant at which the closure is accepted,
not a physical timestamp supplied by the Driver. Every winning **outbound
Call closure**, with or without Retry, retains that original instant, including
response, timeout and abandoned outcomes. Open records and late discarded
observations are not closures and cannot carry a replacement closure instant.
This does not add a timestamp to every transaction, inbound-Call lifecycle or
Session event. A selected Retry failure with another attempt available arms
kind 5 at that closure; the timer reference identifies the possible next
admission. The arm is retained
even if the manager cannot yet consume the completion. A final attempt arms none.

The existing timer schedule can then retain `armed_at = closed_at` and
`due >= armed_at`. Its use is extended to the accepting OUTBOUND closure,
not rewritten at later consumption. This avoids pretending that a due instant
in the past is a newly armed future timer. Expiry merely makes admission
eligible; it cannot bypass unconsumed output, quiescence, capacity or Closing.
The original arm order remains the equal-date tie-breaker.

Recovery must reject a missing closure instant, inconsistent due instant,
wrong timer reference or second arm pretending to renew the delay. Timestamp
retention stays outside the causal seal, as key 56 does today; the existence
and reference of the dated obligation remain causal. No general exact commit
clock or OpenTelemetry export is added by this change. Uniform closure time
replaces C1's earlier Retry-only retention rule; it does not widen timer-arm
eligibility. No measured embedded limitation justified the earlier sparsity.
The additional in-memory representation, padding and recording bytes must be
measured during C2, not advertised as exactly eight bytes per whole record.

## 7. Recovery without recall

Replay/recovery reconstructs these states, without handlers, guests, physical
Drivers or invocation-expression re-evaluation on an already pinned operation:

| Last committed prefix | Reconstructed obligation |
|---|---|
| Opened, not admitted | pending initial admission, same pinned operation |
| Admitted, no OUTBOUND open | the authorized attempt has not been offered; ordinary execution may open it |
| OUTBOUND open, no closure | unknown original Call; never start that old effect again |
| Closed, no Connector transaction | settled completion ready; drafts not yet emitted or accounted as typed envelopes |
| Held, not consumed | exact manager batch pending; do not re-run the Connector transaction |
| Consumed/advanced, not admitted | original due time and next ordinal; no fresh backoff |
| Published/closed | committed routes resume by ordinary cursors; no second emission or publication |

Continuing a deployment and inspecting a diagnostic recording remain different
operations. An unresolved old Call needs the existing explicit timeout or
OutcomeUnknown settlement. Only after that winning closure can `Retry.on`
authorize a distinct new attempt. `idempotent = true` is never itself a command
to recall an open Call. Diagnostic replay never executes any of these pending
obligations. M2 durable storage is not delivered by this algebra.

Original plan/Contract binding must be available and valid before any resume.
Changing source or environment cannot change the retained guarantee, request
or key. A later attempt compares with the operation's prepared material,
not expressions re-evaluated against its new system envelope id. The existing
CallRef derivation and request-digest checks remain independent checks.

## 8. Closing — stop without publishing a held failure

Settled rules: Closing prevents every further attempt admission, invalidates
the wakeup, settles/cancels a currently open Call by the existing protocol and
never transfers work into a successor generation. A response after a winning
closure remains a discarded observation. No operation is kept alive forever
to wait for a cancelled retry timer.

During backoff, Closing terminates the operation with `Close(Stopped)` and
**no new graph output**. The failed batch remains inspectable/accountable
under its original manager disposition. There is no release, re-emission,
new terminal wrapper or retroactive change to a committed route.

If AttemptHeld committed but AttemptConsumed did not, the manager consumes
that exact pending batch and closes Stopped in one transaction, without an
Advance. If it already consumed/advanced, it closes Stopped without consuming
the batch twice. The due obligation becomes inert under the generation fence;
its original arm remains historical evidence. Recovery derives both cases
from the retained prefix, not from the presence of a RAM queue entry.

The same rule covers a selected, non-final winning closure retained while the
owner was live, followed by Closing before the Connector completion commits:
its batch still receives the manager disposition, then is consumed for
Stopped without another admission. Otherwise a crash or delayed projection
could turn the same withheld failure into a public failure. The disposition
is re-derived from the winning closure and the owner state at that prefix.

This does not suppress settlement of a Call that was still open when Closing
began: existing cancellation/response/abandonment and ordinary terminal
publication rules remain. Nor does it retract a final publication committed
before Closing. The relative order of those retained facts decides the race,
never elapsed wall time. An explicit stop is not attempt-bound exhaustion.

## 9. Accounting, policy and inspection

`accounting_observations` currently reads INBOUND envelopes and every
TRANSACTION emission; it ignores raw OUTBOUND completion drafts. Therefore
the retained Connector emissions in AttemptHeld are essential, not optional
diagnostic decoration. Manager consumption is not a second accounting event,
and final routing does not mint or count another copy of its batch.

Correlation must derive operation/attempt/Call from the retained association,
and preserve business origin, instance and any enclosing Loop. It never
attributes an earlier output using a later record. Unknown prices/quantities
remain explicit, decimal/currency and subset rules remain unchanged, and
redaction precedes all display/export for held batches too.

Repository limitation: the HTTP candidate's usage extraction is a downstream
Operator. If an intermediate response is withheld, that Operator does not
run. Merely retaining its JSON body does not produce typed `@measure` values.
Do not claim the existing adapter already measures intermediate attempts;
unsupported extraction remains unknown, not zero. C3 must qualify native
annotated attempt evidence and state this adapter limitation; any additional
extraction surface needs an explicit scope decision, not hidden handler replay.

The debugger projects iteration / operation / attempt separately, including
held outcomes, final publication and original due time. There are no Retry
ports or commands to draw, and no authoring, branches or `run --ui` admission.

## 10. Repository contact and qualification boundary

Read against `c51f165`; line positions are checkpoint evidence, paths and
symbols are the durable references:

| Authority | Verified contact / required change |
|---|---|
| [`loop_protocol::Fact`](../../crates/rastok-core/src/journal/loop_protocol.rs) | Advance requires a CommandRef and state; share the recurrence kernel without fabricating either for Retry |
| [`restore_call`](../../crates/rastok-core/src/runtime/vm/recovery/calls.rs) | compares effect and re-evaluates settings; replace those checks with original plan and pinned-operation authority for attempts |
| [`CallRecord`](../../crates/rastok-core/src/journal.rs) | only Intent/status today; no closure time; Record::timer_schedule excludes OUTBOUND |
| [`TimerSchedule`](../../crates/rastok-core/src/journal/timing.rs) | due must not precede the original arm; arm at closure, never at a later consumer |
| [`CallSemantics`](../../crates/rastok-core/src/connector/call.rs) | visibility table exists; one-case frame cardinality must cover both HTTP terminal response cases |
| [`stdlib`](../../crates/rastok/src/stdlib.rs) | effect present on all six sheets, Process service inherits the common sheet; File guarantee is already fixed |
| [`accounting_observations`](../../crates/rastok-core/src/accounting.rs) | observes emission schemas, not raw OUTBOUND drafts or withheld downstream decoder results |
| [`HTTP usage adapter`](../examples/agent-http-candidate.rkg) | typed usage is emitted by a downstream Operator, not the HttpClient Contract |
| [`delivery-semantics §4.4`](../spec/delivery-semantics.md#44-recovery--a-triage-not-a-gamble) | older recall table conflicts with today's no-recall recovery and the approved target; it cannot authorize old I/O |

C1 is documentation and hand-written trace evidence. C2/C3 still owe executable
independent validation, negative cases, prefix recovery and per-attempt accounting.
C4 owes example/golden migration, Lab/CLI/UI agreement, no_std and measured
memory/code-size checks. No future fact is called qualified by this checkpoint.

## 11. Checks at the initial documentary handoff

15 September, initial handoff `f41ad34`, unchanged executable baseline;
`RASTOK_BLESS` explicitly unset:

```sh
env -u RASTOK_BLESS cargo test -p rastok --test gauge_fixture --test jsonl_fixture --test contracts --test loop_control --test accounting
env -u RASTOK_BLESS cargo test -p rastok --test plan_package --test golden_trace --test v16_exchange_recovery
cargo fmt --all -- --check
cargo doc --workspace --no-deps
git diff --check
```

The eight selected suites pass: **142 tests, zero failures/ignored**. They
preserve the current package, trace, Loop, Contract and accounting baseline;
they are not tests of the unimplemented Retry algebra. Format and rustdoc
checks pass. Local Markdown targets and fenced blocks in the changed documents
were checked; no remote-link or browser audit was run.

Full all-features tests, clippy, no_std builds and embedded measurements were
not repeated for this documentation-only patch. C2–C4 still require their
implementation gates. No Rust, dependency, executable format, fixture or
golden was modified; no external model, paid service, public push or release
operation was invoked.

## 12. Review follow-up and pre-C2 measurement

15 September, executable baseline still `f41ad34`. The review follow-up changes
only documentation. The Loop warning is an explicit C2 obligation (§4.1);
uniform outbound closure time is decided before cutover (§6); Process pending
capacity and retained history have separate measurement obligations (§3.1).

Before adding retained fields, the existing Cortex-M33 echo spike was rebuilt
with `cargo build --release --locked` from `spikes/rp2350-emu` and executed
using its [documented QEMU recipe](spike-rp2350-emulated.md#reproduction).
`llvm-size -A` measured the same release ELF:

| Baseline measurement | Result | Gate / remaining margin |
|---|---|---|
| allocator-tracked peak heap | 111,732 B | 128 KiB / 19,340 B |
| `.text` + `.rodata` | 882,204 + 69,824 = 952,028 B | 1 MiB / 96,548 B |
| record layouts printed by the spike | Record 8 B; Inbound 216 B; Transaction 224 B; Outbound 8 B | layout baseline, not owned payload totals |
| retained records | 67 | `SPIKE_OK`; echo semantic hash unchanged |

The allocator's 256 KiB arena does not change the 128 KiB peak gate. This is
the **existing non-Retry echo workload**, not qualification of Retry batches,
the new closure-time field or recovery peaks. C2/C3 must compare against this
baseline and exercise the larger retained histories explicitly. No gate was
relaxed and no fixture or golden was regenerated.

The initial handoff's 142 tests (§11) were not rerun for this documentary
follow-up; neither were full workspace tests, clippy or rustdoc. The embedded
build/run above is new evidence, not a substitute for those implementation
checks. `cargo fmt --all -- --check` and `git diff --check` pass; fenced blocks
are balanced in the five changed Markdown files and the seven added local
links/anchors resolve. No external-link audit was run.
