# C3 — retained Retry execution and recovery

> **Later qualification — 18 September:** the [common C4 ledger](retry-c4-qualification.md)
> records the single v19/v3 blessing and current checks. The candidate status,
> failures and deferred steps below describe this report's dated checkpoint;
> they are not the current [opening checklist](opening-showcase.md).

15 September 2026. **Implementation checkpoint, not a closed C3 or M1.**
The [C1 algebra](retry-c1-algebra.md), [traces](retry-c1-traces.md) and
[identity inventory](retry-c1-identity.md) remain authoritative. This continues
the [unblessed C2 v19 / recording-v3 candidate](retry-c2-implementation.md).
No package key, causal domain, Contract major or golden is added/blessed here.

At this original checkpoint the source compiler refused Retry at its named execution gate. Tests
and the explicit memory-fixture builder pass through the staged checker,
lowering and independent plan validator. They do not establish blanket source
admission. The later [C4 checkpoint](retry-c4-inspection.md) opens the qualified
Call profile and routes this test matrix through public compilation, with no
new format or golden blessing. Ordinary programs retain their execution paths.

## 1. Invariants implemented at this checkpoint

- `Vm::boot_program` takes the separately transported semantic hash with the
  validated plan. Opened facts bind that original authority. A plan-only boot
  still refuses Retry before effects; the semantic hash cannot be reconstructed
  from a plan body. Hosted run, Lab and diagnostic inspection pass the identity.
- One pure request preparation per operation; prepared bytes, invocation
  settings, partner key, DATA and travelling metadata remain pinned. Each
  recurrence has a new admission and Call identity, with the previous terminal
  as parent. A preparation refusal does not create a Retry operation.
- Winning Call closure retains the delay arm before the completion transaction.
  The first Call's queue timeout is not renewed by manager opening. A recovery
  at 0.25 s admits work originally due at 0.10 s without another wait.
- Whole intermediate stdout/stderr/terminal batches have an explicit Held
  disposition, then Consumed/Advance. They remain in the journal and source
  observations, with no graph delivery. Only Published/Close routes a final
  batch; it reuses its emitted identities.
- Loop and Retry use the existing 1,024-slot recurrence admission bound and
  dated scheduler. An enclosing Loop retains its obligation through Retry
  backoff. Pending-manager capacity is one batch per operation, not one batch
  per configured attempt. Shared mixed-policy scheduling has the additional
  qualification in §8; the remaining close-out gates are listed in §5.
- Closing during backoff consumes a still-pending Held batch if necessary and
  closes Stopped, without a new output. Yoann separately chose ordinary drain
  for a request admitted before Closing whose Retry operation is not opened:
  one Call without Retry, with its original timeout and final publication.
- Recovery binds each packet to its exact original instance, queued admission,
  Contract, Call, complete batch, contexts, routes and retained deadlines.
  No Driver preparation, start, poll or cancellation runs during reconstruction.
  A failed reconstruction leaves the original VM unchanged.

## 2. Corrections found by executable tests

1. **Manager retention is not a `now()` expression.** C2's carrier check
   required `TransactionRecord.logical_time` on a manager fact. That field is
   causal key 54 for an actual semantic clock read. Retry managers execute no
   DSL expression, so their transactions now reject that field. Initial
   preparation time and subsequent admission timer arms already retain the
   necessary instants; no field or clock is invented.
2. **Pinned preparation time is not the opening time of every Call.** C2's
   pure fold required every CallOpen to repeat the initial logical instant.
   The Lab's existing mock `now()` consequently saw zero on every attempt.
   The regression observed `[0, 0, 0]` instead of `[0, 0.1 s, 0.2 s]`.
   Each CallOpen now carries its own opening instant, at or after its admission;
   its closure must match that exact CallOpen. The pin's original preparation
   instant, bytes, settings and key remain unchanged. This corrects C2 §5's
   implementation claim, not the agreed same-operation rule.
3. **A consumed request does not release the enclosing Loop.** Pruning its
   context at Held caused `LoopNoVerdict` during backoff. The opening context
   now lives through the operation's Close; live execution and every recovered
   prefix of the enclosing-Loop fixture agree.
4. **Carrier shape alone does not authorize publication.** A well-shaped
   Published packet attached to an ordinary Operator previously survived spatial
   reconstruction. Recovery now requires the exact retained Connector Call's
   consumption; the hostile regression first failed, then passed.
5. **Equal deadlines follow closure order, not operation hashes.** Reversing
   two winning closures at the same instant exposed hash-sorted readmission.
   Ready Retry work now selects by original deadline and retained closure order.
   The prefix supplies that ordering; no separate authoritative sequence exists.
6. **A refused closure cannot drop an already handed-off response.** The new
   storage-refusal probe first retained only the first Call's open record:
   the Driver had returned its answer, `close_outbound` refused, and the next
   pump had nothing left to close. A second probe covers two answers, including
   a successful closure before the next one refuses. The dispatcher now keeps
   that single batch, its earlier winners and rejected/stray observations,
   without polling another batch or starting more Calls under store pressure.
   It delivers the winners to the VM once the batch settles. This is bounded
   pending handoff, not recovery authority: after restart only the journal
   decides which closures exist, and an open Call is never blindly recalled.
7. **Different recurrence policies must not introduce a timer priority.** A
   mixed Loop/Retry regression first observed `count, call` admissions after
   arming Retry before Loop at the same instant. Both managers now select
   dated admissions against the same head derived from retained timer arms
   and current folds. An earlier Loop is not bypassed by Retry either. No
   ready queue, journal fact, timestamp or format field is added.
8. **A recovered clock is not permission to bypass a retained timeout.** Adding
   an ordinary Call timeout at that same instant exposed a second difference:
   a sweep resumed at the deadline drained recurrences before resolving the
   Call deadline, unlike the uninterrupted sweep. TimeAdvance now processes an
   already-due dated frontier before admitting recurrences. An instance's own
   lifetime waits for its start: the existing successor-generation regression
   caught an early version closing a successor before its start committed.
   §8 narrows that fence to the exact instance rather than all lifecycle work.
   The mixed regression checks all six arming
   orders and resumes each retained cut around the wakeup, comparing complete
   records and outputs, without another preparation/start.
9. **An already-closed Call cannot be cancelled again on recovery.** Counting
   Driver cancellations in the same probe exposed a duplicate after restoring
   the winning closure but before its completion transaction. Before projecting
   a timeout or calling the Driver, expiration now checks the retained closure.
   The old deadline is inert even while completion remains pending. This adds
   no cache, record or new cancellation behavior for still-open Calls.
10. **A restored stop cannot overtake the unfinished dated frontier.** With
    Retry, a fixed lifetime and a timed join due together, recovery immediately
    after the lifetime's Closing record executed `on stop` before the join
    eviction. The uninterrupted sweep did the reverse. A second probe routes
    the eviction through a newly opened child: a blanket priority for starts
    also let that child's start overtake another owner's lifetime expiry.
    Lifecycle work waits for the remaining dated frontier; only a pending
    instance's own lifetime is fenced until its start commits. The two failing
    record-for-record regressions and their qualification are in §8.

## 3. Focused executable evidence

`crates/rastok/tests/retry_runtime.rs` exercises the real VM, retained records
and substituted effect boundary, including:

- final publication after exhaustion and immediate success;
- HttpClient ServerError withholding and File read/replace, including recovery
  of every prefix with no physical HTTP/filesystem observation;
- original queue timeout and closure-based delay;
- stopping a held operation without publication;
- ordinary drain before manager opening, explicit child stop and `single_pass`;
- successor-generation isolation and a hostile missing-operation bypass;
- full recovery at every retained prefix, including Held before Consumed;
- late recovery without preparing the request again;
- Lab time observed on a fresh Call with unchanged request;
- one outer Loop application spanning multiple Retry attempts;
- hostile publication-owner binding and original-program mismatch;
- diagnostic recording-v3 round-trip and every inspected prefix;
- equal-date closure-order selection with deliberately reversed operation ids;
- mixed Loop/Retry/ordinary-Call deadlines in all six arming orders, with
  record-for-record resumption before the deadline and at every retained cut
  around the wakeup at its due instant, and no second Driver cancellation
  for a Call whose closure is already retained.

Two additional `machine_vm` regressions inject storage refusal at Call opening,
closure, and each Retry/Connector transaction. The successful and faulted
two-attempt runs have identical records, outputs and started Call identities;
Held, Consumed and Published each appear once. Partial two-Call settlement also
keeps an earlier rejected answer observable, preserves both winners and leaves
later Driver answers behind the same backpressure boundary. These tests reuse
the existing scripted journal and Driver; no physical effect is called.

The journal protocol also has a focused regression forbidding fake manager
clock reads. The existing non-Retry multiport, Loop, Graph-Call and recovery
suites were rerun after the first runtime integration (123 tests, no failures).
The old Probe test sheet no longer requires removed `effect` metadata.

Qualification results are updated below; the list of tests is not itself a
claim that C3's entire acceptance matrix or all M1 gates are closed.

## 4. Measured Cortex-M33 memory, not an extrapolation

The existing `rp2350-emu` spike has an opt-in `retry-qualification` workload.
The default echo workload and its gates remain unchanged. Both workloads use
the same allocation-level watermark, including transient allocations. Retry
captures both channels, retains all attempts, drops the live VM, then rebuilds
from the journal with no effects or request preparation. This is a core mock
boundary, **not** a physical Process implementation on bare metal.

Pre-copy-removal measurements, bytes:

| Operations | Attempts per operation | Frames per channel per attempt | Bytes per frame | Records | Live peak | Peak including recovery |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 1 | 4 | 4 | 7 | 58,004 | 60,132 |
| 1 | 3 | 4 | 4 | 17 | 76,396 | 78,648 |
| 1 | 8 | 4 | 4 | 42 | 124,272 | 124,272 |
| 2 | 3 | 4 | 4 | 32 | 101,084 | 103,168 |
| 1 | 3 | 1 | 1,024 | 17 | 86,204 | 87,880 |
| 1 | 3 | 16 | 4 | 17 | **137,200** | **137,200** |

The last profile exceeded 128 KiB by 6,128 B. Inspection found an unnecessary
complete transaction clone at Call commit: publication had already prepared
its own emissions, and only the presence of Loop context was needed afterward.
Moving the owned record into the journal instead reduces this exact profile to
**130,560 B**, only **512 B below** the gate. This does not drop records, frames,
observations or destinations. The nine focused runtime tests passed after
that ownership change. Other rows above are explicitly the earlier measurement,
not claimed post-change numbers.

The default echo rerun after scheduling and Closing corrections reports
**112,656 B**, 18,416 B below
128 KiB; 67 records, identical record layouts (8 / 216 / 224 / 8 B) and relay
average 1,916 calibrated ticks. `.text + .rodata` is **1,003,732 B**, leaving
44,844 B below 1 MiB. This is the non-Retry reference image, not a measurement
of every recovery function linked into an application.

The additional Retry + complete-recovery qualification image measures
**1,119,500 B**, **70,924 B above** 1 MiB, after the copy removal, scheduling
and Closing corrections. Its heap peak remains **130,560 B**. Its test Driver
and recovery exercise are linked as well. That is an open footprint result,
not permission to increase a gate or claim that Retry/recovery fits that target.
No allocator arena, target gate or runtime limit was enlarged. These final
reruns include equal-deadline scheduling and ordinary Closing drain. Further implementation
changes require rerunning the measurements before closing C3/C4.

The response-handoff fix above was measured before and after with identical
candidate packages and workload parameters. The new rerun is distinct from
the earlier snapshot's byte counts:

| Image | Heap peak before / after | `.text + .rodata` before / after |
|---|---:|---:|
| Reference echo, 67 records | 112,656 / 112,656 B | 1,003,732 / 1,003,732 B |
| Retry plus full recovery, 3 attempts, 16 frames/channel, 17 records | 130,560 / 130,560 B | 1,119,676 / 1,120,108 B |

Both QEMU runs report `SPIKE_OK` under the unchanged heap gate. The second
image still exceeds the 1 MiB code bound by **71,532 B**; it is not qualified
as fitting that target. These image runs use a healthy MemJournal, not storage
fault injection; they do not measure the additional pending-batch allocation
on refusal. The deterministic refusal behavior is covered by the hosted tests.

### 4.1 Binding sharing and mixed-deadline follow-up, 16 September

The typed-JSON checkpoint subsequently measured 1,004,132 B for echo and
1,120,724 B for Retry/full recovery. Symbol inspection found two copies of
the wide VM expiration path (9,492 and 9,448 B), plus separate Connector
dispatch paths, specialized for the no-Driver and test-Driver bindings.
VM entry points now borrow the existing object-safe
`OutboundConnectorBindings` dynamically. This shares the actual scheduler and
recovery implementations; it removes no verifier, exchange profile or
recovery path, and introduces no dependency or allocation.

Binding sharing alone reduced the Retry image by 17,784 B. After including
the scheduling and inert-Call-deadline corrections above, the final
measurements are:

| Image | `.text` | `.rodata` | Code total | Peak heap | Records |
|---|---:|---:|---:|---:|---:|
| Reference echo | 939,060 B | 71,696 B | 1,010,756 B | 112,656 B | 67 |
| Retry/full recovery, 3 attempts, 16 frames/channel, 4 bytes/frame | 1,022,060 B | 82,208 B | 1,104,268 B | 130,560 B | 17 |

This is a measured trade-off: the reference image grows by 6,624 B, while the
Retry image shrinks by 16,456 B. Echo retains **37,820 B** below 1 MiB; Retry
still exceeds it by **55,692 B**. Neither heap watermark changes, and Retry's
heap margin remains only **512 B**. Echo's calibrated relay average is 1,920
ticks (maximum 2,102); the same package hash and record layouts hold. Both
QEMU workloads report `SPIKE_OK` for their unchanged heap gate. Code-size
qualification remains open; `SPIKE_OK` must not be read as passing it.

These are the same two fixed candidate packages, not re-blessed goldens or a
reduced workload. They do not measure worst-case JSON bodies, store refusal
retention or the entire mixed-policy workload on bare metal.

Reproduce without changing a golden:

```sh
retry_measurement_dir=$(mktemp -d /tmp/rastok-retry-measurement.XXXXXX)
cargo run -p rastok --example retry_spike_package -- "$retry_measurement_dir/retry-3.rkp" 3
cd spikes/rp2350-emu
RASTOK_SPIKE_PACKAGE="$retry_measurement_dir/retry-3.rkp" \
  RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 \
  cargo build --release --locked --features retry-qualification
qemu-system-arm -M mps2-an505 -cpu cortex-m33 \
  -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  -nographic -monitor none -serial none \
  -semihosting-config enable=on,target=native -icount shift=0
```

The fixture builder refuses an existing output file. On this workstation,
Cargo commands use `DEVELOPER_DIR=/Library/Developer/CommandLineTools` locally;
the system's selected Xcode license was neither accepted nor changed.

### 4.2 Shared admission and bounded collection construction, 16 September

The next footprint pass preserves the same packages, features, compiler
profile, workload, gates and complete recovery. It makes two implementation
changes, with no format or identity movement:

- Routing borrows the existing `InstanceAdmissions` seam dynamically. Live
  admission and exact-prefix recovery share the routing code, while each
  retains its own authority. Recovery still cannot consult a later lifecycle
  fact to admit an earlier delivery into a successor generation.
- Conversions of ordered value maps, lexical bindings, resolved policy tables
  and metadata-presence sets insert directly into their destination trees.
  The linked image previously contained separate bulk-collection sorting
  implementations and transient buffers for these conversions. The conversion
  of materialized values into expression bindings is shared by its existing
  callers. Canonical key order, field membership, first-error order and
  shadowing precedence do not change. No general replacement collection or
  sorting algorithm is introduced.

Measured stages on the Retry/full-recovery workload:

| Stage | `.text + .rodata` | Difference from previous stage |
|---|---:|---:|
| §4.1 baseline | 1,104,268 B | — |
| Shared live/prefix admission | 1,088,724 B | −15,544 B |
| Expression maps and shared value bindings | 1,075,916 B | −12,808 B |
| Remaining measured map conversions | 1,050,728 B | −25,188 B |
| Metadata-policy set construction | **1,040,824 B** | −9,904 B |

Final images at this checkpoint:

| Image | `.text` | `.rodata` | Code total | Peak heap | Records |
|---|---:|---:|---:|---:|---:|
| Reference echo | 900,716 B | 71,940 B | **972,656 B** | 112,656 B | 67 |
| Retry/full recovery, 3 attempts, 16 frames/channel, 4 bytes/frame | 958,620 B | 82,204 B | **1,040,824 B** | 130,560 B | 17 |

Both unchanged workloads now satisfy the 1 MiB code gate: **75,920 B** margin
for echo and **7,752 B** for Retry/full recovery. The respective reductions
from §4.1 are 38,100 B and 63,444 B. Both QEMU runs report `SPIKE_OK`, and
neither heap peak changes. Retry still has only **512 B** below 128 KiB:
this is not headroom for unmeasured annotation tables or refused-store batches.
Echo's relay average is 1,907 calibrated ticks (maximum 2,089). Package hashes
and record layouts are unchanged. Preliminary cold/inlining hints did not
provide a useful reduction and are absent from the patch.

The three new equivalence probes cover multi-key/nested materialized bindings,
mutation isolation, rejection of unresolved payloads, metadata round trips
and metadata first-refusal ordering. Full-suite and target-check results for
this snapshot are recorded in §6;
the size result alone does not close Retry or M1.

## 5. Closing clarification and remaining qualification

Yoann resolved the pre-opening drain question in
[D-2026-09-15-4](../spec/decisions.md#d-2026-09-15-4--drain-an-unopened-retry-request-as-one-ordinary-call):
one ordinary Call without Retry. The regression first failed with
`InvalidRetryProtocol(OriginalBinding)`. Both explicit child Closing and
`single_pass` now publish the ordinary complete final batch without opening
an operation or admitting a recurrence. Recovery checks the exact retained
Closing prefix and the ordinary invocation settings; it cannot infer this
exception from a missing Retry packet. The initial drain tests and their
every-prefix recovery pass. No C1 Retry fold admission rule is weakened.

The mixed-timer, capacity, concurrent-key and Closing cuts now have the
additional executable evidence in §8. Process job, File read/replace and
HttpClient remain the only admitted Call profiles; their existing retained
attempt/recovery probes are complemented there, not replaced. This is a
bounded acceptance matrix, not a claim to enumerate every workload.

Still required, without silently expanding scope:
- remaining per-attempt coverage and adapter migration. Typed HTTP output now
  retains original token/amount, signed quantity and refund samples in both
  Held and Published batches, with every-prefix recovery and recording proofs.
  [The retention ledger](annotation-types-m1.md#15-retention-without-reports)
  replaces the report obligation under D-2026-09-16-7. A downstream extractor
  still cannot observe a held response; the HTTP gallery must use the declared
  response type. Absent usage on timeout is not zero;
- Retry inspection/UI and gallery migration; the debugger must not invent
  Retry ports, commands or a RetryExpired wrapper;
- final image/heap measurements and one explained v19/v3 golden blessing.

M1 remains open on these items and the independent opening gates in
[opening-showcase.md](opening-showcase.md). No public tag, publication, paid
benchmark or external reviewer run is authorized by this checkpoint.

## 6. Check results, with snapshot boundaries

The complete `cargo test --workspace --all-features --no-fail-fast` run at
the initial C3 integration snapshot reports **1,443 passed, 9 failed, 0
ignored**, across 50 groups including empty binary/example/doc groups.
That snapshot contains nine runtime tests, before the later diagnostic,
tie-order, HTTP/File and Closing additions. The nine failures are the
candidate identity movements already listed at C2, not waived tests:

| Group | Failures | Unblessed movement |
|---|---:|---|
| library | 2 | M0 semantic hash, diagnostic recording-v3 retained rows |
| Contracts | 2 | effective idempotence/defaults and versioned network Contracts |
| gallery | 1 | the explicitly changed HTTP 500 ServerError fact |
| gauge fixture | 1 | package v19 header; pure body/hash/trace still match |
| JSONL fixture | 2 | Contract-major semantic hash and package bytes; causal trace still matches |
| package | 1 | echo's Contract-major package bytes |

The **follow-up focused runtime suite had 16 passing tests**. It includes every
prefix for Process, HttpClient, File read/replace and Closing, with no recalled
effect. The independent Retry protocol's 15 tests pass as well. These later
focused runs are not relabelled as another complete workspace run.

`cargo fmt --all -- --check`, strict all-target/all-feature workspace clippy,
`cargo doc --workspace --no-deps`, and `git diff --check` pass. Core checks on
`thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
`riscv32imac-unknown-none-elf` pass after the Closing change. An initial typo
selected uninstalled `riscv32imc-unknown-none-elf`; the correct configured
target above was then checked, without installing or changing a toolchain.
The two QEMU profiles and exact ELF measurements are recorded in §4.

No golden or gate was changed to obtain these results. JavaScript/browser
qualification and independent protocol campaigns were not rerun; this slice
does not edit the UI or any physical Driver.

Storage-refusal follow-up: **83 `machine_vm` tests**, **16 `retry_runtime`
tests** and **356 core tests** pass, including the two regressions that first
lost the handed-off responses. Strict Clippy, formatting, rustdoc and the same
three no_std targets pass. The before/after images in §4 are measured after
this functional correction. The eight new flat-type source tests and six
site-contract tests also pass in the combined tree; annotation interpretation
and typed response hydration are separate gates at that snapshot.

The subsequent typed-JSON slice adds a passing seventeenth `retry_runtime`
test: annotated quantities and provider costs from held and published attempts
are counted exactly once, including after recovery of each prefix. It does
not change the Retry protocol, request descriptor or journal. Updated image
measurements, including the still-red code-size result, are retained in
[the hydration ledger](annotation-types-m1.md#7-existing-output-json-hydration).

The 16 September typed-JSON combined workspace run (flat types, response-handoff
fix and typed HTTP responses) is **1,475 passed, 9 failed, 0 ignored**, across
52 groups including two empty doctest groups. The failed targets and reasons
are exactly the six groups / nine unblessed movements in the table above.
Formatting, strict workspace Clippy, rustdoc, the no-default-feature build
and the three configured core no_std targets pass. No fixture is re-blessed,
ignored or removed; the complete workspace is not yet green.

The mixed-deadline follow-up passes **83 `machine_vm`**, **18 `retry_runtime`**
and **7 `typed_json_response`** tests after all three corrections, including
the existing successor-generation regression. Strict workspace Clippy,
formatting, rustdoc, the no-default-feature build and all three configured
core no_std checks also pass at this final code snapshot. The image reruns in
§4.1 include these corrections.

The complete follow-up run reports **1,476 passed, 9 failed, 0 ignored**,
across the same 52 groups. Its nine failure names match the earlier hydration
checkpoint exactly; no test or golden was changed to remove a failure.
An initial cold run instead reported 1,475 passed / 10 failed: the extra
local WebSocket gateway probe could not reach its listener within its existing
30-second startup bound, with no child output. The identical probe then
passed in 0.19 s, and the complete rerun above passed it too. No startup bound,
fixture or assertion was relaxed. The candidate workspace is still not green
because the nine inventoried v19/v3 movements remain unblessed.

The shared-admission/collection footprint pass adds three equivalence tests.
The complete Cargo run reports **1,478 passed, 10 failed, 0 ignored**, across
52 groups. The extra failure is the same local gateway startup probe: no
listener or child output within its existing 30-second bound. After the CLI
executable loaded, the identical probe passed in **0.21 s**. A sequential
rerun of **all 50 already-built test executables**, without rebuilding,
filtering or ignoring a test, reports **1,479 passed, 9 failed, 0 ignored**.
The two empty doctest groups passed in the Cargo run. The remaining nine
failure names exactly match the inventoried candidate movements above; no
golden, fixture or timeout was changed to obtain the rerun result.

At this snapshot, **364 core**, **83 machine_vm**, **18 retry_runtime** and
**7 typed_json_response** tests pass. The final test-only name clarification
was also checked by another 364-test core run. Formatting, strict workspace
Clippy, rustdoc, the no-default-feature build and all three configured core
no_std targets pass. The unchanged-package QEMU evidence and independent
code-size totals are in §4.2. Public CLI checks retain echo's candidate hash
`878a34b7…` and gauge's unchanged `0d3ad43d…`. No browser/JavaScript campaign,
external protocol qualification or release action was run in this pass.

## 7. Measurement retention without a report — 17 September

D-2026-09-16-7 removes the M1 report requirement without changing Retry's
protocol or recording. The three quantity/amount/refund regressions now inspect
the original Connector transactions: ordered Held/Published emission ids,
attempt ordinals, exact typed values and empty spatial routes for held data.
Consumed creates no new emission. Every diagnostic-recording and recovery
prefix retains exactly those records without request preparation or Driver
starts. All **20 retry_runtime** tests pass.

The [retention qualification](annotation-types-m1.md#qualification--17-september)
records the full workspace result (**1,504 passed / 9 failed / 0 ignored**),
unchanged embedded footprint and unavailable browser pass. The nine failures
remain candidate identity/Contract/golden expectations, not hidden or ignored
tests. C3/C4 and M1 remain open on the matrix and migration items in §5; a
computed cost total is no longer one of them.

## 8. Mixed timers, bounded publication and exact instances — 17 September

This follow-up adds five runtime regressions and strengthens the existing
backoff-stop test. It changes no plan, Contract, retained record, identity
domain or golden. The public Retry source gate remains closed until the
remaining migration and qualification gates above are met.

The mixed Retry/join/fixed-lifetime test first fails for arming order
`retry, window, waiting`, at recovered prefix 14: the next record is
`accumulator / l:stop` instead of `pair / l:evict`. Routing the eviction
through a previously unopened `forward` Operator exposes the complementary
failure: at prefix 14 for `retry, waiting, window`, `forward / l:start`
overtakes `accumulator / l:closing`. The final correction fences only the
lifetime of the exact instance whose start is pending. Starts and stops of
other instances cannot jump ahead of the rest of the dated frontier. No new
scheduler state or retained clock fact is introduced.

| Probe | Evidence |
|---|---|
| Retry, timed join and fixed lifetime at 1 ms | All six arming orders, with direct eviction output or first admission through a relay; resume before the sweep and at every retained cut around the shared deadline; identical complete records and outputs, no Driver preparation/start/cancel |
| Retry wake versus Closing drain or settlement | Wake at 1 ms or 2 ms, both arming orders; every retained cut resumes identically; settlement publishes one ordinary `OutcomeUnknown`, not a new Retry; reconstruction invokes no effect |
| Closing in backoff | Lifetime before or exactly at the Retry due instant prevents another admission; no held failure becomes public |
| Stop after closure, Held, or Consumed | All three cuts consume the original batch exactly once and close Stopped without publication; every subsequent retained prefix resumes identically |
| Final batch at bounded fan-out | 1,024 frames reach both recipient mailboxes atomically; 1,025 refuse before emission, routes or Close commit; a repeated attempt and recovered closure preserve the same refusal and original facts |
| Two keyed instances, two admissions per origin | Four distinct operations, six Calls: Alice's two operations retry, Bob's two finish once; every prefix restores without effects, and a late response is discarded without a new record |

These are deterministic substituted-Driver probes. They do not claim physical
network qualification, worst-case annotated embedded payloads, or UI agreement.
The tested final-capacity refusal is not a promise that an oversized batch
will eventually fit: the invariant is no partial delivery or lost operation.

The unchanged Cortex-M33 Retry/full-recovery workload retains 17 records,
the same package hash and `SPIKE_OK`. Before this scheduling correction:
`.text` 964,620 B + `.rodata` 83,724 B = 1,048,344 B. After:
`.text` **964,644 B** + `.rodata` **83,724 B** = **1,048,368 B**, a 24 B
increase and **208 B** below the unchanged 1 MiB gate. Peak heap remains
**130,560 B**, **512 B** below 128 KiB. This is measured narrow headroom, not
permission for unmeasured growth or a raised threshold.

The reference echo image is also rerun, using its unchanged package:
906,732 B text + 73,440 B rodata = **980,172 B**, **68,404 B** below 1 MiB;
peak heap **112,656 B**, 67 records, unchanged candidate semantic hash and
`SPIKE_OK`. Its relay averages **1,905 calibrated ticks** over 32 rounds
(maximum 2,088). This is a separate workload, not extra capacity for Retry.

Checks on the final exact-instance lifetime fence:

- Focused `retry_runtime`: **25 passed**; `machine_vm`: **83 passed**, including
  the existing successor-generation lifetime regression. No ignored tests.
- Strict workspace/all-targets/all-features Clippy, `cargo fmt --all -- --check`,
  `git diff --check`, workspace rustdoc and the no-default-features check pass.
- Core compiles on `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
  `riscv32imac-unknown-none-elf`.
- JavaScript: **18 passed**, the 15 layout and three lexical editor checks.
  No new browser, optional full-tokenizer or external protocol pass is claimed.

The final complete `cargo test --workspace --all-features --no-fail-fast`
pass reports **1,509 passed / 9 failed / 0 ignored**, 55 groups. The nine
failures are exactly the candidate identity/Contract/golden expectations from
§6; they remain failures, not waived or re-blessed tests.

Two earlier complete passes each report **1,508 passed / 10 failed / 0 ignored**.
Before the final lifetime fence, the live WebSocket CLI fixture misses its
five-second listener-start deadline: `Connection refused`, child still running,
no stderr. After the final fence, the resident-gateway fixture misses its own
30-second bind deadline with empty stdout/stderr; the CLI fixture passes.
The resident gateway then passes all three tests in an isolated rerun. The
last full pass uses the same final binaries without rebuilding and both
gateway fixtures pass. Neither deadline is changed. Unusually slow executable
startup was observed, but its cause is not established and these cold-start
incidents are not erased by the warm result.

Local evidence: `/tmp/rastok-retry-timers.sT5iIw/`, including
`workspace-tests.log`, `workspace-final.log`, `workspace-warm.log`,
`gateway-recheck.log`, `exact-start-fence.log`,
`exact-fence-clippy.log`, `exact-fence-rustdoc.log`, `exact-fence-nostd.log`,
`exact-fence-no-default.log` and `javascript.log`. No causal/plan/recording golden
or publication asset is modified by this follow-up.
