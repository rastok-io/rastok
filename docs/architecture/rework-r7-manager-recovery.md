# R7 — manager contexts and spatial recovery

> Work in progress, 2026-09-07. The latest checkpoint (§8) opens fuel-bounded
> pure-Operator Graph Loops, including the App, with exact boundary admission,
> delivery contexts, quiescence and recovery. Wider Graph obligation profiles
> and complete Worker recovery remain gated.

The second checkpoint (§5) qualifies outbound Call, join, lifetime and Closing
recovery. Earlier checkpoint counts and gates below are retained as history,
not substituted for the current matrix in §8.

## 1. Invariant and repository contact

A Loop packet is part of one causal transaction, not a freestanding claim
about a run. Its carrying Executor must belong to the exact run instance
scope; manager facts belong to that run's owner; routed contexts refer to
deliveries actually retained by that transaction; exhaustion is published as
the same ordinary `Loop.expired` emission.

Three executable probes contradicted this invariant before the change:

- changing a Loop transaction's owner to the enclosing App still passed the
  independent r2 replay fold;
- adding a context for a nonexistent delivery still passed;
- deleting the public expiry emission while keeping the Expired fact still
  passed.

The protocol-only fold had verified command identities, ordinals and phase
transitions, but did not inspect their carrying record. These are journal
acceptance/replay defects, not a claim that the normal Worker produced such
records. The regressions first failed because replay accepted the forgeries.

## 2. Record-aware fold

`Fold::apply_transaction` is now the acceptance path for MemJournal, independent
replay and the existing persistent-Operator recovery path. It checks the
packet and its record in one scratch fold, publishing neither on refusal.
The lower-level `Fold::apply` still checks packet algebra in isolation;
it is not a journal-acceptance API.

The fold derives active `InstanceRef -> parent InstanceRef` links from the
existing `l:start`/`l:stop` records. It does not infer scope from a lexical path
prefix or a business origin. Two tenants with identical owner paths remain
distinct, and a successor parent generation cannot repair a child's link to
the preceding generation. Context stacks follow these retained links from
outermost to innermost; repeated runs and reversed nesting refuse.

For each r2 record:

1. inherited context runs must be active and enclose the transaction instance;
2. each manager fact must name the carrying owner instance;
3. each routed context must match exactly one retained
   `delivery_ref(edge, envelope_id)` occurrence;
4. an Expired fact must have exactly one matching `Loop.expired` emission,
   with index zero and the exact retained envelope.

Refusals are typed: `ContextOwner`, `ContextDelivery`, `ExpiryEmission`, plus
the existing protocol errors. A failed check leaves records, pending commands
and the cached fold unchanged. Recovery performs the same check in its
candidate VM before replacing the live VM.

The cached projection is created lazily when the first r2 packet is accepted,
seeding its parent links from the committed prefix. Later lifecycle changes
update it atomically. A journal without r2 work retains no new index; an
explicit fold query may derive the lifecycle links on demand. These are
disposable indexes, not another journal or new source of generations.

## 3. Evidence and identity

The three integration regressions check **both** independent replay and actual
MemJournal commit, require exact error variants, and prove atomic refusal.
They cover every manager/application r2 row in a fuel-exhausting run and four
expiry-publication forgeries: missing output, wrong port, wrong index and
duplicate output.

Four core tests exercise valid descendant contexts, generation isolation,
two keyed tenants at the same lexical paths, stack order and repeated runs,
and exact edge/envelope occurrence matching. The existing pending-command
recovery test now includes its actual parent/start facts rather than presenting
isolated manager packets as a complete causal record history. No test was
ignored or dropped. Existing integration tests still restore every committed
prefix and compare all subsequent records without rerunning producers.

No source, package, Contract, record-field or causal recipe changes. The parent
links already exist in lifecycle facts; key 55 already carries the r2 packet.
Echo, JSONL and gauge candidate bytes and all scenario seals are checked against
the preceding checkpoint; no golden is re-blessed.

Executed checks on this checkpoint:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check`, `git diff --check` | pass |
| workspace/all-targets/all-features clippy, warnings denied | pass |
| `cargo doc --workspace --no-deps` | pass |
| `cargo test --workspace --all-features --no-fail-fast` | **1,162 passed, 32 failed**, 31 suite reports, zero ignored |
| core / v16 Loop suites | **310 / 25 passed** |
| no-std: `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` | pass |
| candidate echo/JSONL/gauge ledger | exact package bytes and every causal seal unchanged |
| emulated Cortex-M33 | `SPIKE_OK`; **106,244 B** peak, **24,828 B** below the 128 KiB gate |

The failing test-name multiset is unchanged: 25 old Loop/Graph/UI-form tests
and seven categorical expectations. Seven new regressions pass; no test or
golden was removed to obtain that result.

The embedded witness was rerun before the first core edit and after it, with
the same candidate package and 67 records. Heap peak and 6,440 B after open
are unchanged. Relay timing remains 1,462 / 1,415 / 1,656 ticks
(average / minimum / maximum), calibrated against 39,999 ticks per 2,000,000
instructions. Decode changes from 21,270 to 21,269 ticks. Text 738,276 B plus
read-only data 64,688 B totals **802,964 B**, an increase of **6,056 B**, below
1 MiB. The non-Loop witness allocates no context index; these measurements do
not qualify the future Graph Loop workload.

## 4. Remaining execution gates

This scope check is **not** complete delivery-context validation or quiescence.
The following still require implementation and end-to-end evidence together:

- retain the exact opening Graph-boundary admission before expansion, so its
  internal fan-out opens one run rather than one run per flattened edge;
- propagate and re-derive per-delivery contexts from consumed positions,
  including push/pop at Loop boundaries and rejection of cross-context joins;
- derive `Loop.state` and commands from that exact consumed context, not merely
  from lexical containment;
- count run-scoped pending deliveries, Calls, timers and child-manager work,
  excluding downstream work after a boundary pop and the selected command itself;
- gate command consumption and recurrence on that complete obligation set;
- restore Graph admissions and the remaining temporal/exchange obligations from
  retained facts, then qualify all three host traces from R0.

The source and independent plan gates still refuse Graph Loop orchestration.
Passing low-level nested-context tests does not override those gates. The full
workspace also retains its old-form and categorical-fixture migration failures.

## 5. Temporal and outbound Call recovery — 2026-09-07

D-2026-09-07-3 wires the accepting producers of key 56 and restores their
finite obligations. Admission and producer transactions retain the original
join, delivery-budget, instance-lifetime/renewal and Closing dates in actual
arm order. A single pure route helper selects both the retained schedules and
their post-commit scheduler projections. Recurrent Call admissions retain the
new request's timeout; no deadline is reconstructed as `resume + duration`.
Handshake and inbound respond-Call producers also retain their dates, but
their exchange recovery remains explicitly refused.

Recovery uses a scratch VM and a read-only lifecycle prefix fold. Looking up
the final Journal generation had routed an early arrival into a later
generation; the new query follows only records already visited. Each record
reconstructs its selected targets, immutable instance bindings, private state,
cursor consumption and deadlines before subsequent records may use them.
Residual deliveries consumed by `l:stop` must match exactly; a retained drain
and the stop's consumed positions establish settlement without replaying a
physical cancellation. A single-pass start also retains its derived pending
close when a crash interrupted the following close-request commit.

An OUTBOUND open restores its held delivery and claim. A retained response
restores its typed completion; Timeout/Abandoned need only the Contract's
pure scheduler projection. No old request is prepared or started again and
no record is appended by reconstruction. The subsequent completion transaction
is checked against those retained outputs before consuming the request.
A queued request already beyond its timeout cannot start an effect when the
caller drains the recovered VM before calling `advance`.

The original implementation armed fixed/tumbling lifetimes at start, despite
the specified creating-admission deadline. They now arm at admission, as idle
already did. A waiting successor keeps its original date, even when it becomes
eligible after that date. The scheduler does not move logical time backwards
or extend the deadline on promotion. Removing the incorrect lifetime arm
claim from `l:start` is a **named timed-lifetime causal move**, not a semantic
hash or package change. The successor-generation regression now expects its
already-due close rather than an unearned fresh lifetime.

Eight new integration tests cover ingress/producer schedule retention,
held and settled Calls, pending recurrence, timeout-before-start, timed-join
candidate expiry and consumption, repeated generations, idle renewal,
fixed/tumbling dates, and bounded Closing with residual mailboxes. Two new
machine tests cover a failed single-pass close commit and hostile prefixes:
missing/changed deadlines, altered Call closure, open before owner start, and
backwards retained time. Those forgeries refuse without changing the live VM
or journal. A core acceptance regression requires observable `logical_time`
to equal the schedule's arm instant. The targeted tests were first run red
for the defects they exposed; no existing golden or test was ignored.

| Check executed on this checkpoint | Result |
|---|---|
| fmt, `git diff --check` | pass |
| workspace/all-targets/all-features clippy, warnings denied | pass |
| `cargo doc --workspace --no-deps` | pass |
| workspace/all-features, no-fail-fast | **1,173 passed, 32 failed**, 32 suite reports, zero ignored |
| v16 recovery / v16 Loop | **8 / 25 passed** |
| no-std, the same three ISA targets as §3 | pass |
| candidate echo/JSONL/gauge ledger | every package byte, semantic hash and causal seal unchanged |
| Cortex-M33 | `SPIKE_OK`; **111,452 B** peak, **19,620 B** below 128 KiB |

The failing test-name multiset is identical to §3: 25 old Loop/Graph/UI forms
and seven categorical expectations. Eleven new tests pass. No golden was
re-blessed. The embedded witness still opens at 6,440 B and retains 67 records;
relay average/minimum/maximum is **1,871 / 1,820 / 2,046** ticks with the same
39,999-tick/2,000,000-instruction calibration. Text 771,436 B plus rodata
66,020 B = **837,456 B**, under 1 MiB. The 5,208 B heap increase includes
retaining real Call schedules on this witness; no history is dropped.

**Not closed:** Graph-boundary opening/context/quiescence from §4,
Event/inbound-Call/Session recovery, the three-host R8 matrix and R9 migration.
Passing temporal recovery does not make Graph Loop executable. All those
admission gates remain explicit.

## 6. Exact consumed contexts and Call verdicts — 2026-09-07

Recovery now derives an application context from its exact consumed delivery,
including the request held across an OUTBOUND Call. Merely belonging to the
same owner instance is insufficient: two active runs in that instance cannot
exchange contexts. A Loop entry cannot be consumed, or its Call opened, before
the retained run admission. Missing consumption, omitted context and a context
transplanted from the other admitted run all refuse as `InvalidRecovery`, with
the live VM and journal unchanged.

Call recovery additionally re-derives the verdict from the retained completion
and resolved route selection. This uses the existing pure Contract completion
path, not a Driver operation. A forged `Again` cannot replace the natural Done
of a successfully completed Call whose source routes request no recurrence.
Reconstructing the held request neither starts nor cancels a physical effect.

Two machine regressions exercise the real compiler and public recovery API.
They first demonstrated accepted missing consumption and accepted Call opening
without run admission, then passed with the checks wired. Their other hostile
prefixes remain valid to the protocol-only fold, proving why exact spatial
consumption and Contract projection are necessary in addition to packet
algebra. The valid interleaved two-run prefix and both Call opening/completion
prefixes recover successfully before the corresponding mutations are tested.

No package, semantic or causal identity changes. This qualifies existing
Operator/Call consumption; it does not implement Graph boundary push/pop,
cross-context joins or Graph quiescence from §4.

## 7. Window closure qualification and current matrix — 2026-09-07

The pure Graph-window regression exposed a live-state loss: an aggregator's
`on stop` product reached a sibling formatter only after that formatter had
stopped. Reopening it yielded **7 instead of 27**. Equal-length node names,
tested in both lexical orders, keep canonical address ordering from hiding
the defect. The scheduler now preserves possible recipients of pending stop
products until those products have drained. Graphs remain scopes: no Graph
handler, application transaction or mailbox is introduced.

A nested version exposed a second refusal: the committed route named a leaf
whose intermediate Graph had not yet started, so the closing parent could
not admit that ancestor. The journal now verifies the exact retained route's
cause, ancestor keys/time boxes and parent generation. A different key, an
external producer, or a reservation predating that ancestor's previous stop
cannot authorize admission. This is a proof from existing records, not a
new capability bit or ambient owner-path inference.

Four variants cover opposite node orders, lazy nested routing, and fixed and
tumbling lifetimes. Each recovers before expiry, checks preserved formatter
state and stop ordering, then recovers the closed prefix without additional
outputs or stops. This qualifies the pure window path, not every bounded
Closing/exchange profile. D-2026-09-07-4 names the affected closing traces and
results; no package format or semantic hash moves.

| Check executed on §§6–7 | Result |
|---|---|
| fmt, `git diff --check` | pass |
| workspace/all-targets/all-features clippy, warnings denied | pass |
| `cargo doc --workspace --no-deps` | pass |
| workspace/all-features, no-fail-fast | **1,178 passed, 32 failed**, 32 suite reports, zero ignored |
| core / v16 Loop / v16 recovery | **313 / 25 / 9 passed** |
| no-std, the same three ISA targets as §3 | pass |
| candidate echo/JSONL/gauge ledger | every package byte, semantic hash and causal seal unchanged |
| Cortex-M33 | `SPIKE_OK`; **111,452 B** peak, **19,620 B** below 128 KiB |

The failing test-name multiset is identical to §5: 25 old Loop/Graph/UI forms
and seven categorical expectations. Five new tests pass across §§6–7; none
is ignored and no golden is re-blessed. The embedded witness still opens at
6,440 B and retains 67 records. Relay average/minimum/maximum remains
**1,871 / 1,820 / 2,046** ticks with the same calibration. Decode takes
21,271 ticks. Text 775,892 B plus rodata 66,272 B = **842,164 B**, an increase
of 4,708 B over §5, still under 1 MiB. The heap watermark is unchanged.
That persistent non-Loop witness does not measure the window-closing or
Graph-Loop workload.

Graph Loop admission/context/quiescence, Event/inbound-Call/Session recovery,
the full three-host R8 matrix and R9 migration remain open and fail-closed.

## 8. Pure Graph Loop admission and quiescence — 2026-09-07

D-2026-09-07-5 opens one fuel-bounded Graph Loop whose body consists of
finite Operator port handlers. A public App Loop and a nested Loop Graph
both cross source, v16 serialization, Registry-free validation and the VM.
The Graph owns no application handler, synthetic Node or executable mailbox.
Its attached manager retains the opening admission under the existing Worker
bound before expanding the boundary into its internal fan-out. Twelve arrivals
on one edge consume in FIFO order, not run-hash order. Two admissions with the
same business origin open distinct runs, while one admission fanning out to
two children still opens only one run.

`Open` retains the exact admission; the following `Admit` consumes its ordinary
cursor and commits all selected internal routes and their exact r2 contexts.
Subsequent child transactions inherit context from the delivery they consume,
not an owner-path guess. `Loop.state` remains the application's opening envelope
even after an intermediate Operator overwrites DATA. Egress pops the context;
downstream parent work is no longer an obligation of that run. Routed Next,
Again and explicit Done, and natural terminal boundary crossings, produce
commands in the child transaction. Pending sibling deliveries postpone command
consumption, and obligations of another run cannot prevent this run closing.

Delay starts at command consumption after quiescence. Recovery retains the
original due date, restores a committed unconsumed command without rerunning
its producer, and restores an initial admission without injecting another
root. Exhaustion publishes `Loop.expired`, including directly on an App Loop.
Closing discards pending admissions/commands and invalidates a delayed reentry
without migrating it to another generation.

The independent plan proof composes possible verdict counts over the ordinary
DAG, saturating at two. A transaction coalesces its own terminal crossings;
different child transactions do not. Every Dispatch path must have exactly
one verdict. The route validator independently verifies the selected type of
Next against the Graph entry: changing Again(text) to Next(text) in a plan
whose entry is int now refuses. No serialized productivity or control claim
substitutes for these derivations.

Fourteen new integration regressions cover:

- every committed prefix of the three-application, two-child reference run;
- concurrent runs, a same-origin fan-out, FIFO admission and bounded precommit
  capacity; the 1,025th pending admission appends no INBOUND;
- inherited state after DATA rewriting, all three commands, natural terminal
  output, App expiry, delay and Closing after recovery, and current-instant
  work preceding a future `advance` deadline;
- refused storage commits at every boundary, followed by the same successful
  record sequence;
- hostile omitted admission cursors/contexts, a substituted verdict, and a
  manager consumption moved ahead of its sibling's work. These histories pass
  the protocol-only fold but refuse spatial recovery without changing the live
  VM or original journal.

Four existing source-checker tests are updated for the actual opening of this
gate: the contributed expiry now needs an explicit disposition, the pure Graph
plan validates, and nested Loop binding resolution remains source-visible but
its execution still refuses independently. No old-form test is ignored or
deleted and no golden is re-blessed.

**Named profile limits:** exactly one Loop owner in the plan; positive authored
fuel, not a Graph measure certificate; no Connectors, joins or lifecycle
handlers; no classified/annotated state, values or ports; no descendant-local
stop/lifetime; and no immediate boundary-only verdict. Descendant commands use
output routing, not direct command statements. A descendant cannot bypass the
entry to obtain a context. These paths refuse by name at plan validation;
they do not silently run with weaker quiescence or policy semantics. Operator
and standalone outbound Call Loop profiles remain unchanged. Their existing
measure and temporal-recovery evidence is not evidence for these wider Graph
profiles. R8 and R9 remain open, and the UI stays paused.

**Identity:** no field, tag, Contract, domain or package version is added. Key 11
already retains the boundary inventory, key 55 the r2 packets and key 56 the
deadlines. This first Graph profile now produces its specified admission,
context and manager facts; they are not old Graph-handler traces reinterpreted.
The separate echo/JSONL/gauge candidate ledger remains the nonaffected control.

| Check executed on §8 | Result |
|---|---|
| fmt, `git diff --check` | pass |
| workspace/all-targets/all-features clippy, warnings denied | pass |
| `cargo doc --workspace --no-deps` | pass |
| workspace/all-features, no-fail-fast | **1,192 passed, 32 failed**, 32 suite reports, zero ignored |
| core / v16 Loop / v16 recovery | **313 / 37 / 9 passed** |
| no-std, the same three ISA targets as §3 | pass |
| candidate echo/JSONL/gauge ledger | package files compared byte-for-byte; every semantic hash and causal seal unchanged |
| Cortex-M33 | `SPIKE_OK`; **111,452 B** peak, **19,620 B** below 128 KiB |

The failing test-name multiset is identical to §7: 25 old Graph/Loop/UI forms
and seven categorical expectations. Fourteen new tests pass. The new graph
admission/command projection is absent on the embedded control workload.
It still retains 67 records and opens at 6,440 B; relay average/minimum/maximum
is **1,887 / 1,831 / 2,073** ticks, decode 21,274 ticks, with the unchanged
39,999-tick calibration per 2,000,000 instructions. Text 830,556 B plus rodata
68,248 B = **898,804 B**, 56,640 B above §7 and below 1 MiB. These are linked
code and relay measurements, not a Graph Loop workload memory qualification.

An earlier mixed-build qualification run had four extra CLI startup failures
because concurrent default-feature builds replaced the all-feature CLI under
test. The final workspace run used one consistent all-feature binary; those
four tests passed without changing their source or timeout. The evidence above
uses that completed run, not the transient mixed-artifact result.
