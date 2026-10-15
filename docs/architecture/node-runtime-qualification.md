# Node realignment — runtime organisation (lot 2)

Status: 15 October 2026, lot 2 implemented and locally qualified. This closes
the runtime-organisation lot of D-2026-10-11-1, after the
[lot 1 closure and Fable follow-up](node-realignment-qualification.md).
It changes neither semantics nor package, journal or recording formats.
The human-review registers remain owned by Yoann and Fable. This is local
qualification, not a new independent review or an external opening gate.

## Responsibilities

Core `runtime::execution::inspection` contains the read-only instance,
mailbox, delivery, Call and Session projections and `EnvelopeView`. Its
`Execution` methods read private execution state; moving them into a child
module requires no new mutation API or enlarged scheduler visibility. An
owned snapshot is independent of later steps and is not recovery authority.
Hosted `ui/view.rs` retains formatting, redaction, causal navigation and JSON
presentation. Calls and Sessions remain distinct. Call direction is derived
from `CallReferenceInspection`, not duplicated in a second field; the UI
still emits the same inbound/outbound label. Session direction remains data
because a `SessionId` alone does not describe who opened it.

Hosted `worker::Worker` owns one Execution, MemJournal and RuntimeBindings.
Driver binding completes before it constructs either mutable authority.
Its read-only accessors serve recording, the Lab and the UI. `run` owns the
continuous host loop; `run_until_idle` polls Drivers and runs ready work
without waiting or advancing time; `step` performs at most one ready
execution step. Private `process_ready` implements the hosted polling pass;
crate-private `settle_action` and `poll` preserve the Lab's batch and
safe-pause protocols. Logical time is an explicit input. There is no new
task, thread, clock, lease or parallelism.

Idle does not mean completed or stable. An open Call, Session or future
deadline can require a new external observation or time advance while no
execution work is ready. `run_until_idle` returns then; `run` continues
listening under the existing CLI completion rule. `Execution::is_stable`
keeps its stronger existing meaning. The `worker::Host` substitution contract
supplies input, explicit logical time, observations and the waiting policy;
the CLI and deterministic Worker tests provide its two implementations.

The CLI still owns stdin, the real time source, output rendering, recording
checkpoints, logs and the existing 10 ms idle wait. The Simulator still owns
authored actions, assertion windows, mock diagnostics and its suspended time
advance. Neither duplicates the Worker's Driver/transaction processing.

`shutdown` consumes the Worker, releases physical Drivers and returns its
journal. It does not create Closing, cancellation or terminal records.
`instance.stop` and lifecycle completion retain their existing semantics.

## Control interface

| Control | Returns when |
|---|---|
| `run(&mut host)` | App input has ended, no bound source requires polling, and no Call or Session is open; otherwise the host keeps listening |
| `run_until_idle(observe)` | Non-blocking Driver polling makes no further progress and no execution work is ready; time is unchanged |
| `step()` | At most one ready execution step has run; `false` means none was available, not that execution is finished |
| `shutdown(self)` | Physical bindings have been released and the owned journal returned; no logical closure is synthesized |

`accept_input` and `advance_to` remain the explicit controls for embedders.
`execution()` and `journal()` are read-only. The continuous host contract
separates `Poll::Pending` from input EOF, and keeps recording after committed
work but before public output presentation. Errors distinguish input,
admission, time advance, processing and checkpoint failure so the CLI keeps
its existing error-path recording behavior.

## Ordering preserved

The three existing control paths are intentionally not made identical:

- A hosted pass admits at most 64 inbound proposals, runs ready work,
  offers inbound responses and timeouts, collects terminal observations,
  polls outbound Drivers, reports rejections, then runs ready work again.
- A batch Lab pass offers one source proposal and its inbound terminals,
  runs ready work, then polls outbound Drivers. It settles at most 64 such
  passes and performs the established final ready-work pass. If the bound
  is exhausted and that final pass also fails, both faults remain visible
  in that order.
- Interactive Lab polling finishes the preceding outbound batch before
  offering the next source frame. It does not run a graph transaction.
  In particular a write acknowledgement cannot move behind the next
  scripted frame merely because the user selects single stepping.

Faults stay typed in the Worker; the CLI and Lab render them at their own
presentation boundary. A processing fault does not imply that earlier records
were rolled back. The single-step fault regression checks the exact
pre-commit prefix and its reserved input.

## Verification

The final API in `b7670ff` passes the complete gate with Rust 1.97.1:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --workspace --all-features --no-fail-fast` | 1,598 passed, 61 groups, zero failures or ignored tests |
| `cargo doc --locked --workspace --no-deps` | Passed, no warnings |
| Real Chrome, `browser.test.cjs` | 35 passed, zero failures, cancellations or skipped tests; 54.65 s |
| Layout, editor and chat-client JavaScript | 32 passed, zero failures or skipped tests |
| Core `no_std`, three targets | Passed on `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and `thumbv8m.main-none-eabihf` |

The detached-snapshot regression and 12 UI projection checks pass, including
Retry inspection at every recorded prefix. All five Worker API regressions
pass: exact journal/output/inspection equivalence through continuous, idle,
scripted and step controls; pending input versus EOF; an open Call surviving
idle and reaching its timeout after EOF; fault/checkpoint stage preservation;
and shutdown without invented closure. The complete suite also exercises real
HTTP/WebSocket listeners, distinct resident children, File effects, recording
and recovery. No paid model call is part of these checks.

The browser suite exercises live, historical and replayed snapshots, keyed
runs, held Retry attempts, recording, redaction, command disposition,
assertions, keyboard navigation and playback. Diagram 06 was regenerated
from PlantUML and its rendered SVG was visually checked. Browser and
JavaScript commands use the already-installed tooling, without installing a
dependency:

```sh
RASTOK_PLAYWRIGHT=/Users/steelcoder/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright \
RASTOK_CHROME='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' \
  node --test crates/rastok/src/ui/browser.test.cjs
node --test crates/rastok/src/ui/graph_layout.test.cjs \
  editors/vscode/rastok-dsl/grammar.test.cjs examples/agent-chat.test.cjs
cargo check --locked -p rastok-core --no-default-features --target thumbv7m-none-eabi
cargo check --locked -p rastok-core --no-default-features --target riscv32imac-unknown-none-elf
cargo check --locked -p rastok-core --no-default-features --target thumbv8m.main-none-eabihf
```

Network tests run after compilation, without a concurrent build. Several
newly built executables spent minutes in macOS `_dyld_start` before Rust
started; that startup wait is distinct from the tests' reported durations.
No timeout or system security setting was changed. Auxiliary harness calls
with `--list` execute no test body and are not counted as passing tests.

Against lot 1's `35d3072`, no example, fixture, golden, package, causal trace
or recording changed. Both protected human-review registers are unchanged.
The public echo check still produces semantic hash
`562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813`.

## Local delivery

| Commit | Scope |
|---|---|
| `05b67a6` | Fable's lot 1 follow-up: source-level Plan-depth regression, exclusive-bound diagnostic and live documentation headers |
| `ab0ae1c` | Read-only inspection module, nonredundant Call direction and detached-snapshot proof |
| `b7670ff` | Shared hosted Worker, continuous/idle/step controls, CLI/Lab integration and focused proofs |

This close-out records the final complete gate and aligns the live status
documents. The independent review, release archives and other external
opening gates remain in the [release inventory](release-qualification.md).
No push, tag or publication was performed.

## Embedded recheck — 15 October

Both unchanged workloads printed `SPIKE_OK`. These are ordinary images,
without the optional stack probe. Heap and `.text + .rodata` are exactly
unchanged from the lot 1 reference:

| Workload | Decode peak heap | Execution/recovery peak heap | Code | Heap margin to 128 KiB | Code margin to 1 MiB |
|---|---:|---:|---:|---:|---:|
| Echo, 32 inputs | 36,160 B | 111,404 B | 969,012 B | 19,668 B | 79,564 B |
| Retry, 1 operation, 3 Calls, 16 frames/channel, 4 bytes/frame, full recovery | 30,684 B | 129,188 B | 1,040,912 B | 1,884 B | 7,664 B |

Rust 1.97.1, the spike's existing release profile, target
`thumbv8m.main-none-eabihf`. From `spikes/rp2350-emu`:

```sh
env -u RASTOK_SPIKE_PACKAGE -u RASTOK_RETRY_OPERATIONS \
  -u RASTOK_RETRY_FRAMES -u RASTOK_RETRY_FRAME_BYTES \
  RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
qemu-system-arm -M mps2-an505 -cpu cortex-m33 \
  -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  -nographic -monitor none -serial none \
  -semihosting-config enable=on,target=native -icount shift=0

env RASTOK_SPIKE_PACKAGE=/tmp/rastok-lot1-close.Lzhv5R/retry-3.rkp \
  RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 \
  RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' \
  cargo build --locked --release --features retry-qualification
```

Run the same size and QEMU commands after the Retry build. The referenced
package is the unchanged lot 1 workload (941 B); no new package is generated
by this lot. `llvm-size` is LLVM 18 at
`/opt/homebrew/opt/llvm@18/bin/llvm-size`; QEMU is
`/opt/homebrew/bin/qemu-system-arm`. Echo sections are 898,748 B `.text` and
70,264 B `.rodata`; Retry sections are 959,892 B and 81,020 B.

Stack is not remeasured in this lot. The dated lot 1 probes remain the
reference, including the accepted level-255 package's 79,168 B stack and
107,328 B decode heap. No embedded limit is relaxed.

## Deferred time value type

The `LogicalTime(i64)` value type is deliberately not implemented in this
lot. Yoann assigned it on 15 October to A-T1-12, with the relative logical
timeline and separate civil origin of D-2026-10-10-1; the
[roadmap](../../ROADMAP.md#later--incident-lab-and-safe-dogfood) records that
handoff. Existing time carriers and semantics remain unchanged.
