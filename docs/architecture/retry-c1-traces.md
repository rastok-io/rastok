# C1 — reference traces for Retry

15 September 2026. Hand-written obligations for the
[C1 algebra](retry-c1-algebra.md), **not executable test results**.
The [identity inventory](retry-c1-identity.md) specifies their proposed wire
forms. C3 must turn these into prefix-recovery tests; C4 must exercise the
same stories through the Lab and diagnostic replay.

## 1. Fixture and notation

The fixture below describes a future validated plan, not a program that the
v18 CLI accepts. It has exactly one HttpClient Call Connector, `pay`:

| Property | Value |
|---|---|
| owner | `pay`, instance `I = (address, generation 0)` |
| entry | `in`, the Contract's resolved request type |
| request | one immutable prepared POST, digest `D`, partner key `K` |
| policy | idempotent true; attempts 3; on `{ServerError, TimeoutExpired}` |
| timing | Call timeout 1 s; fixed Retry delay 100 ms |
| capture | bounded JSON response; one `out` for Completed/ServerError, none for TimeoutExpired |
| routes | App.in → pay.in; pay.out → App.reply; pay.exit → App.result |
| root outputs | reply accepts the body; result accepts the entire ten-case amended HTTP terminal union |

All productible outputs have a disposition, including the cases not selected
by Retry. There is no observer, hidden sink, Loop port or undeclared sibling.
The first two traces share this fixture. Other traces state their differences.

`O` is the operation derived from exact opening delivery `d0` and `I`.
`C1`, `C2`, `C3` are distinct Calls, not compilation slices. `e1.out` and
`e1.exit` name the two actual envelopes of attempt 1. Lower-case `r0`, `r1`,
etc. label rows for discussion; **row numbers are not identity material**.
Times below are logical seconds, not measured execution time. Ordinary
INBOUND/root routing and `l:start` facts preceding `d0` are prerequisites,
not counted as retry-created transactions.

## 2. Server error, crash before consumption, then success

At time 0, `d0` is admitted. Its existing Call budget is due at 1.0 s;
preparing/opening the operation must not reset it.

| Row | Time | Record | Retained fact / observable result |
|---|---:|---|---|
| r0 | 0 | TRANSACTION, manager | OperationOpened(O, d0, I, D, K), AttemptAdmitted(O, 1); no consumption of d0 and no Driver effect yet |
| r1 | 0 | OUTBOUND | C1 open, linked to O/1; only after this record may the Driver start |
| r2 | 0.010 | OUTBOUND | C1 wins ServerError(503), with a body; retain closure time 0.010 and kind-5 arm due **0.110**, possible admission 2 |
| r3 | 0.011 | TRANSACTION, Connector | consume d0 once; emit e1.out and e1.exit; AttemptHeld(O, 1, C1, exact batch), both destined to manager; no graph delivery |
| crash | — | no record | lose only volatile state, after r3 and before manager consumption |
| restore | 0.250 | no invented record | O/1 settled; batch pending; original timer is overdue, not renewed |
| r4 | 0.250 | TRANSACTION, manager | AttemptConsumed(O, 1, batch) and Advance(O, 2, due=0.110) together; no second emission |
| r5 | 0.250 | TRANSACTION, manager | AttemptAdmitted(O, 2), after quiescence/capacity; fresh internal request e2.in, timeout due 1.250 |
| r6 | 0.250 | OUTBOUND | C2 open from that exact admission; D and K unchanged, CallRef different |
| r7 | 0.260 | OUTBOUND | C2 wins Completed(200), with a body; retain closure time; no Retry timer |
| r8 | 0.261 | TRANSACTION, Connector | consume admission 2; emit e2.out and e2.exit; ordinary routes to App.reply/result; AttemptPublished and Close(final) atomically |

Expected evidence:

- two Calls, one operation, one downstream body and one downstream terminal;
- intermediate envelopes remain retained and inspectable, with an explicit
  manager disposition, not `nothing`;
- the new request's DATA/travelling metadata and prepared bytes/key are pinned;
  its system id is new, its parent is e1.exit, its origin is unchanged;
- no old Call is started during restore, and no invocation expression is
  re-evaluated to prepare C2;
- being 140 ms late does not extend the backoff to 0.350 s;
- accounting visits each typed emitted envelope once, including held evidence;
  raw JSON with no applicable usage schema remains unknown, not zero;
- a late second response for C1 is discarded, not attributed to C2.

## 3. Attempt bound: the last failure passes through

Use three timeouts and no response-body drafts. The manager has capacity and
consumes each batch at its closure time; equality of these logical times does
not fuse the records.

| Attempt | Admission / Call open | Winning closure | Connector disposition | Next admission |
|---:|---:|---:|---|---:|
| 1 | 0.000 | 1.000, TimeoutExpired | emit exit to manager; then consume and Advance | 1.100 |
| 2 | 1.100 | 2.100, TimeoutExpired | emit exit to manager; then consume and Advance | 2.200 |
| 3 | 2.200 | 3.200, TimeoutExpired | emit exit to App.result and Close(final), same transaction | none |

The graph sees exactly one `TimeoutExpired`, the envelope produced by attempt
3. There is no fourth Call, no extra failure emission, no `RetryExpired`, no
Loop command and no retry wakeup after attempt 3. The first two timeout exits
have their own retained ids and consumed manager dispositions.

With `Retry.attempts = 1`, the first timeout is final: no intermediate batch
or delayed admission exists. With an unselected terminal, publication is
final even if attempts remain. Closed unsafe selections still fail policy
validation; a bound of one does not introduce a second policy language.

## 4. Closing during backoff — approved no-publication rule

Take the prefix of trace 2 through r3: ServerError(503) is held, timer due
0.110. `instance.stop` targets **I**, not all instances of pay.

| Prefix when Closing wins | Required retained continuation | Forbidden behavior |
|---|---|---|
| closure retained, Connector transaction not committed | commit its ordinary batch with manager disposition; consume for Close(Stopped) | turn queue timing into public ServerError |
| held batch, consumption not committed | one manager transaction consumes that exact batch and closes Stopped, without Advance | lose the batch, re-run its producer, or publish it |
| consumed/advanced, admission still waiting | Close(Stopped); no repeated consumption; due wakeup is inert | start attempt 2 or move it to generation 1 |
| Close(Stopped) already committed, then recovery | reconstruct closed operation; history/accounting still includes the original failure | synthesize any additional out/exit |

No authored destination is reserved to stop this waiting operation. The
manager receipt is bounded and settled; it is not an orphan obligation keeping
the parent in Closing. A later admission to a successor instance opens a new
operation with that successor generation, never inherits this one's timer.

At equal logical instants, retained record order resolves the race. If the
Closing fact precedes AttemptAdmitted(2), there is no second attempt. If
AttemptAdmitted/CallOpened wins first, the existing active-Call cancellation
and terminal-settlement rules apply. This decision neither erases already
published outputs nor suppresses the existing settlement of a still-open Call.

## 5. One Loop application contains several Retry attempts

Composition fixture: one Loop Graph `refine`, terminal `out`, fuel 2, input
`{ round: int }`; its child `pay` has the policy above. The complete wiring is:

```text
refine.in -> pay.in
pay.out -> nothing
pay.exit -> judge.in
judge.next_state -> Loop.next
judge.finished -> refine.out
refine.out -> parent's terminal result
refine.err (LoopExpired) -> parent's terminal failure
```

The `judge` Operator accepts every amended HTTP terminal. On a Completed
outcome it requests Next(round=1) when `Loop.state.data.round` is 0, otherwise
emits the terminal result. All other outcomes emit the terminal result as a
failure value. Its two outputs are fully disposed; it supplies exactly one
Loop verdict. The expanded fixture must declare those types in C3.

| Enclosing Loop | Retry operation | Retained progression | What judge sees |
|---|---|---|---|
| application 1, round 0 | O1 | attempt 1 ServerError held → 100 ms → attempt 2 Completed published | only Completed; one Next command |
| application 2, round 1 | O2, distinct opening delivery | attempt 1 Completed published | one terminal result, natural Done |

The outer fuel pays for **two applications**, not three Calls. During the
first backoff O1 remains an outstanding obligation of application 1. It cannot
produce LoopNoVerdict or release that application early. O2 may prepare a new
operation from round 1; only O1's *attempts* must preserve O1's request/key.
There is no `Loop + Retry` on one owner and no Retry containing Loop.

## 6. Isolation and exact opening identity

For two keys, Alice's attempt 1 times out and Bob's succeeds immediately.
Their operations bind to distinct instance references. Bob receives one
terminal, is never called again and does not block Alice's backoff. Stopping
Alice during that backoff produces no new Alice output and changes nothing
about Bob's already closed operation.

A second fixture sends two distinct fan-out deliveries of **one envelope**
to the same Connector instance. They share origin, input EnvelopeId, request
digest and possibly partner key, but have different opening DeliveryRefs.
They must derive different operation ids, CallRefs and subsequent admissions.
Retry is not local business deduplication. A forged attempt referring to the
other delivery, key, generation or operation is rejected before I/O.

## 7. Crash-prefix and sabotage matrix for C3

The same retained prefixes must reconstruct identically in ordinary replay
and recovery; only explicit continuation may execute new eligible work.

| Cut or forgery | Required result |
|---|---|
| after operation opening, before admission | original pin and admission pending; no invented Call |
| admitted, no Call open | authorize at most that attempt's first Call open |
| Call open, no winning closure | unknown old Call; never recall it, even if idempotent |
| closure, before emission | reconstruct completion, original closure time and due obligation |
| held batch, before consumption | exact batch pending, no repeated Connector transaction |
| consumption/Advance, before due admission | original deadline; one admission when eligible |
| final closure, publication blocked on a full mailbox | no output/Close committed until all routes fit |
| final publication/Close, before downstream consumption | recover ordinary deliveries, no second final emission |
| Closing in any waiting prefix | trace 4; no new graph output or successor inheritance |
| omit/substitute an emitted id in AttemptConsumed | refuse; exact whole-batch receipt required |
| consume twice, skip attempt ordinal, admit before receipt/due | refuse independently of manager RAM |
| change prepared bytes/settings/key or original Contract | refuse before new effect; current source is not authority |
| renew closed_at/due at manager consumption | refuse inconsistent retained schedule |
| response arrives after timeout or after another response won | existing discarded observation, never another closure |
| same timestamp for Closing and wakeup | retained precedence, no scheduler-order guess |
| full bounded manager receipt/admission budget | precommit pressure, no hidden drop or unbounded allocation |

HTTP classification tests must include 499/500/599 and another admitted status
above 599 if the physical parser admits one; 500 with decode failure is still
DecodeFailed, not ServerError. Both usable response cases owe one body frame.
Missing/extra frames and a status inconsistent with the selected case refuse.

## 8. Review additions before C2

The Loop warning is required, not a new unresolved design choice. These are
prospective tests of `W-LOOP-REPEAT`, not diagnostics the v18 binary emits:

| Loop path | Checker result at cutover |
|---|---|
| Again repeats a possibly visible Call with idempotent false | warning naming owner, Again site and Connector; Loop remains admitted |
| same path, idempotence omitted | same warning; absence is no guarantee |
| Again caused by a BeforeEffect failure, but repeats a successful non-idempotent sibling | warning for that sibling |
| every potentially repeated Call proved BeforeEffect on that path | no warning |
| potentially visible repeated Calls all idempotent true | no warning |
| only an intentional Next path, not Again | no warning from this Again-specific rule |
| same unsafe selected terminal under Retry without the guarantee | error, not this advisory warning |
| reorder independent Connector declarations / closed arms | same warning subjects in canonical order |

Uniform closure-time retention also needs a **non-Retry** fixture. Run one
ordinary Call to a response, one to timeout and one to abandonment. Each
winning OUTBOUND closure retains its original logical instant; none has a
Retry packet or retry arm. Missing time refuses; open records have none.
Read-only replay restores those instants without I/O. A later discarded
response cannot change them. The ordinary causal seals stay unchanged even
though the v3 recording bytes acquire the retained time.

For Process capacity qualification, use stdout and stderr together, many
small frames and large frames near the raw aggregate budget. Instrument both
pending receipt load and journal-owned memory. For attempts 3, the two
intermediate batches coexist in **history**, never as two simultaneously
unconsumed manager batches of the same operation. Delay consumption to show
that the next attempt cannot start; resume it to show receipt capacity is
released without deleting the historical batch. Repeat with concurrent
operations and prefix recovery. Frame count/decoded representation/metadata
overhead must be measured, not assumed equal to captured bytes.

Accounting qualification needs known typed usage on intermediate and final
emissions, missing usage as unknown, subset consistency, decimal totals,
per-operation correlation and redaction. Retaining a body is not evidence
that a downstream decoder ran on it. These are tests to implement, not
results inferred from these tables.
