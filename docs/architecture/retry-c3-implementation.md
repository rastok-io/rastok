# C3 — retained Retry execution and recovery

15 September 2026. **Implementation checkpoint, not a closed C3 or M1.**
The [C1 algebra](retry-c1-algebra.md), [traces](retry-c1-traces.md) and
[identity inventory](retry-c1-identity.md) remain authoritative. This continues
the [unblessed C2 v19 / recording-v3 candidate](retry-c2-implementation.md).
No package key, causal domain, Contract major or golden is added/blessed here.

The source compiler still refuses Retry at its named execution gate. Tests
and the explicit memory-fixture builder pass through the staged checker,
lowering and independent plan validator. They do not establish blanket source
admission. Ordinary programs remain on their existing execution paths.

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
  per configured attempt. Shared mixed-policy scheduling still needs the
  qualification listed in §5.
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
   pure fold required every Intent to repeat the initial logical instant.
   The Lab's existing mock `now()` consequently saw zero on every attempt.
   The regression observed `[0, 0, 0]` instead of `[0, 0.1 s, 0.2 s]`.
   Each Intent now carries its own opening instant, at or after its admission;
   its closure must match that exact Intent. The pin's original preparation
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
- equal-date closure-order selection with deliberately reversed operation ids.

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

Still required, without silently expanding scope:

- complete mixed Loop/Retry scheduling qualification, including equal-time
  ordering against other dated obligations and original-prefix recovery;
- hostile/full-capacity publication and commit-failure matrix, concurrent exact
  instances, all admitted Call profiles and closure races;
- per-attempt accounting correlation and explicit unknown quantities. C1's
  native annotated-attempt test cannot be claimed from the existing downstream
  HTTP extractor: held responses do not execute it. Current source Connectors
  cannot declare their own typed ports (`check.rs::node_body`), and the fixed
  Process/HTTP raw output schemas contain no token/cost annotation. Do not add
  extraction or label status/exit codes as token quantities to satisfy a test;
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

The **current focused runtime suite has 16 passing tests**. It includes every
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
