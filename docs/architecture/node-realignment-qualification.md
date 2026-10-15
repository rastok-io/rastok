# Node ontology realignment — qualification

Status: 15 October 2026. This records lot 1 of
[D-2026-10-11-1](../spec/decisions.md#d-2026-10-11-1--realign-the-plan-and-the-runtime-on-the-specifications-node-ontology-before-the-opening).
The vocabulary follow-up is `2259fd4` and `beabf68`; the Execution interface
is named in `90c646f` and `9218779`. Yoann's instruction to close lot 1 approves
the measured boxed-body layout and the package-only regeneration inventoried
in `54c2b3e`. The current representation is format 21; the final closure gate
is recorded at the end of this report. D-2026-10-14-1 resolves the last depth
question: reader budget 256, expression and type bounds unchanged at 128.
Lot 1 is closed: the boundary-package stack measurement and the full
workspace gate pass, with 1,591 tests across 61 groups, none failed or ignored.
The Operator-containment step (`5513883`) moves state and handlers into the
Operator body. `dabda8e` puts the App's port declarations on its
root Graph. Scope chains now belong to the validated plan rather than each
Execution. Those qualified steps preserved the separate declaration tables.
The unified Node table and indexed wiring are in `59f1ddf`; indexed queued
deliveries are in `dd5bda4`. Earlier sections remain dated checkpoint evidence,
including failed candidate checks; they do not describe the final status.
Inspection-module separation and the hosted Worker are now qualified in the
separate [lot 2 closure](node-runtime-qualification.md); the dated checkpoints
below retain their original scope and results.

## Invariants before changing the representation

A route belongs to exactly one Graph. That Graph determines which face of
a boundary port the route names:

- When the endpoint is the Graph holding the route, its input is a source
  and its output is a destination: the internal face.
- When the endpoint is a direct child Graph, its input is a destination
  and its output is a source: the external face.

The root App is a Graph with ordinary boundary ports, not an absent Node.
A single typed Node index will locate a declaration; it will not identify
an instance, a delivery or a journal fact. Identity recipes continue to use
paths and exact instance generations, never table positions. Composing routes
must preserve every crossed Graph scope for instance selection, lifetimes
and Loop contexts. Manager commands remain non-spatial destinations.

## Existing-representation measurements

The fixed Retry workload was regenerated in a temporary directory by the
existing `retry_spike_package` example, then decoded and run by the
Cortex-M33 spike. It has one operation, three attempts, sixteen frames per
channel and four bytes per frame. The run includes reconstruction from the
journal and checks that recovery does not prepare or start another Call.

| Measurement at `beabf68` | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap immediately after package decoding | 7,364 B | 5,652 B |
| Live heap after execution | not sampled | 93,464 B |
| Live heap after recovery | not sampled | 93,328 B |
| Allocator-observed peak | 112,656 B | 130,560 B |
| Margin below the 128 KiB heap gate | 18,416 B | 512 B |
| `.text` | 902,836 B | 960,460 B |
| `.rodata` | 70,160 B | 80,852 B |
| Code gate measurement, `.text + .rodata` | 972,996 B | 1,041,312 B |
| Margin below the 1 MiB code gate | 75,580 B | 7,264 B |
| Journal records | 67 | 17 |
| QEMU result | `SPIKE_OK` | `SPIKE_OK` |

The Retry package's semantic hash is
`d8e89d6dc345e7c6199ee5ffc5664b682e09040a8589c6e934016bbe1c32581f`.
Package decoding took 15,892 emulated ticks under `-icount shift=0`;
this is an instruction-count proxy, not a hardware latency measurement.
Echo's hash is
`562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813`;
its decoding took 21,555 emulated ticks.
The allocator arena is 256 KiB; the acceptance gate remains **128 KiB of
measured peak use**, not the arena's size. These are workload measurements,
not upper bounds for every valid program.

Reproduction, from the repository root:

```sh
fixture_dir=$(mktemp -d /tmp/rastok-node-realignment.XXXXXX)
cargo run --locked -p rastok --example retry_spike_package -- "$fixture_dir/retry-3.rkp" 3
cd spikes/rp2350-emu
RASTOK_SPIKE_PACKAGE="$fixture_dir/retry-3.rkp" \
RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 \
RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' \
cargo build --locked --release --features retry-qualification
qemu-system-arm -M mps2-an505 -cpu cortex-m33 \
  -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  -nographic -monitor none -serial none \
  -semihosting-config enable=on,target=native -icount shift=0
llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
```

## Isolated Node-body comparison

The optional `node-layout-probe` feature of the spike allocates four copies
in turn: the current declaration tables, one inline-body Node table, one
boxed-body Node table, and one Node table with private typed body tables.
The allocator samples each copy's resident allocation and construction peak;
the probe compares every copied declaration field before dropping it and
checks that the allocation is released. The original validated plan remains
unchanged and executes the usual workload after the probe.

**This first comparison is not a new decoder or a qualification of a complete
layout.** Routing, resolved-port policy and derived execution indexes are
excluded equally from these declaration-only copies. The Graph body has an
empty wiring vector in this first sample; actual wiring is not claimed to cost
zero. The second sample below includes wiring and indexes. Code size of the
instrumented image is not a production-layout measurement.

| Copied declarations, allocator bytes on Cortex-M33 | Echo | Retry |
|---|---:|---:|
| Current owner + executor + boundary + handler tables | 3,240 | 2,552 |
| Single Node table, inline bodies | 3,192 | 2,580 |
| Single Node table, boxed bodies | 3,044 | 2,504 |
| Single Node table, compact side tables | 3,044 | 2,504 |

Construction peaks equal the resident samples in these two small cases.
The Node row is 248 B inline, 152 B boxed or with a body-table index; its
common header is 144 B. The stack-resident vector headers occupy 12 B for
either single-vector layout and 48 B for the side-table layout; these are
not measurements of maximum stack depth. A handler without its redundant
owner field is 160 B rather than 168 B on this target.

Counting each Operator's handlers before allocation matters. A first probe
used `filter().collect()`, which reserved four entries for echo's single
handler: 480 extra bytes, enough to consume almost the entire Retry margin.
The measured version allocates the known declaration count, with unchanged
contents. This is evidence about construction, not a new runtime policy.

The small Retry fixture has only a root Graph and a Connector. Saving 48 B
there does **not** justify selecting a layout before measuring reference
wiring, derived routes, load-time peaks and the complete runtime. The
current preference to investigate is boxed bodies: in these samples they
match the side tables without adding another body-index lookup. That is
an experimental candidate, not the layout decision.

Reproduce the comparison by adding `node-layout-probe` to the build features
above. For echo, unset the four `RASTOK_SPIKE_*`/`RASTOK_RETRY_*` overrides
used above and build with `--features node-layout-probe`. The probe also
reports the current decoder's peak: 35,988 B for echo and 30,384 B for Retry.
The instrumented runs still end in `SPIKE_OK`, with the original hashes,
67/17 records and the original 112,656/130,560 B whole-run peaks.

## Combined wiring experiment

The second sample translates the validated reference wiring to index-based
endpoints and stores each rule in its declaring Graph. It also allocates the
composed routes, physical edges, scope-chain cache and execution-order index.
Composition uses the existing core algorithm; its temporary allocations are
included in the construction peak. Physical endpoint/scope sets are compared
against the existing plan's edges. Discards and commands do not become edges.

The control includes its three current routing tables and its scope-chain
cache. Resolved-port policy, assets and metadata schemas are excluded on both
sides. Constructing the experiment's input inventory is outside the sampled
interval: that is fixture preparation, not a claim about a new package loader.
The converted declarations, reference wiring and derived indexes stay alive
together while measuring resident allocation.

| Declarations + wiring + indexes, resident / construction peak | Echo | Retry |
|---|---:|---:|
| Current tables and scope chains | 3,608 / 3,616 B | 2,824 / 2,832 B |
| Inline bodies | 4,084 / 4,892 B | 3,268 / 3,984 B |
| Boxed bodies | 3,936 / 4,744 B | 3,192 / 3,908 B |
| Compact side tables | 3,936 / 4,744 B | 3,192 / 3,908 B |

Including wiring changes the conclusion of the declaration-only sample:
boxed bodies save declaration bytes but the combined copy grows by 328 B
for echo and 368 B for Retry. Subtracting 368 B from the current 512 B margin
would leave only 144 B **if all other runtime allocations stayed equal**.
That subtraction is a sensitivity estimate, not a measured new-VM peak.
No savings from indexed mailbox deliveries have been assumed.

Boxed bodies remain the candidate to implement and test: they match the side
tables here without an extra body-table lookup. They are not yet a qualified
production choice. A new decoder, nested programs and the actual execution/
recovery memory and code measurements must precede that choice. Both
instrumented runs finish in `SPIKE_OK`, with the original semantic hashes and
67/17 records; these remain executions of the original plan after the
experimental copies have been dropped.

This still does **not** qualify final code size, a new decoder or execution
under new Node indexes. In particular, equality of sorted physical-edge sets
does not prove unchanged delivery order. The original VM executes after the
copies are dropped. Those limits remain explicit until the production
representation and its differential tests are implemented.
The two fixed embedded workloads are flat. Their composed-edge scope and
parent-chain samples do not qualify nested Graph passages, Graph Loops or
keyed re-entry; those need the gallery's nested programs as well.

## Qualification still required

The 512 B margin is not permission to increase the gate. Compare the three
physical-layout candidates named by the decision: inline heterogeneous
bodies, boxed bodies, and compact side tables behind one Node-facing model.
Measure package-load peak, resident plan and derived-index memory, complete
execution/recovery peak and executable code size. Measuring only `size_of`
or omitting route composition at load would not qualify a layout.

The final comparison must also retain the echo workload and the existing
functional tests: identical admissions, emissions, causal identities,
dispositions and recovery results. Causal traces stay unchanged. Package
witnesses change only with the separately inventoried representation change;
none were regenerated for these measurements.

The remaining qualification is not limited to replacing the row types:

- Preserve the relative order of executable Nodes when Graphs are inserted
  into the unified table. The scheduler's deterministic ordering must not
  change merely because index numbers change. Today
  `compiler/lower.rs::sorted_nodes` sorts by **kind, then path**, not by
  lexical scope preorder. The existing owner and executor orders therefore
  cannot be treated as interchangeable. The chosen row order or private
  execution-order index must preserve the observable scheduling order;
  account for that index if it is needed.
- Derive physical edges once per source/destination pair, not once per case.
  Keep manager commands and instance closure out of that edge table.
- Retain crossed Graph scopes when composing routes. A flat endpoint pair
  alone does not describe keyed admission, idle lifetime activity or Graph
  Loop context.
- Measure the declared wiring and the derived indexes together, including
  transient allocations at load, instead of charging only the final vectors.
- Derive the mailbox port from its edge index, but keep journal positions,
  routes and identity recipes path-based. Recovery must resolve these facts
  through the validated plan, never interpret old numbers as new indexes.
- Qualify nested Graph boundaries, case fan-out, Loop and Retry, source
  Connectors and exact-instance closure against the existing traces before
  updating package witnesses. The package movement gets its own inventory.

## A-T4-4 — admission preparation and installation

A-T4-4 lands separately in `22ba272`, in `runtime/vm/admission.rs`: pure, fallible
preparation; journal commit; then infallible installation. Its five explicit
source cases are App boundary, Event, inbound Call, Session opening and Session
message. Call and Session opening share source-instance reservation; a
Session opening's exact id is obtained before commit. The existing order of
refusals, handshake non-delivery and message ancestor selection are preserved.
This does not settle the human review's open Session ancestor-generation or
handshake-recovery questions.

Two focused tests cover a refused ingress commit with no partial Call/Session
installation and identical retry identity, and a malformed prepared Session
opening refused before any commit. The focused VM, inbound HTTP and exchange
recovery groups pass: 92 tests, no failures. The first test also passed on the
unsplit implementation, establishing the behavior the refactor preserves.

The normal spike images below contain no `node-layout-probe`. They include
the additional decoder-peak print used by the qualification instrumentation.

| After A-T4-4 | Echo | Retry with recovery |
|---|---:|---:|
| Heap after decoding / decoder peak | 7,364 / 35,988 B | 5,652 / 30,384 B |
| Whole-run heap peak | 112,656 B | 130,560 B |
| Heap margin | 18,416 B | 512 B |
| `.text` | 902,060 B | 961,236 B |
| `.rodata` | 70,120 B | 80,828 B |
| Code total | 972,180 B | 1,042,064 B |
| Code margin | 76,396 B | 6,512 B |
| Code change from `beabf68` | −816 B | +752 B |
| Journal records | 67 | 17 |
| Result | `SPIKE_OK` | `SPIKE_OK` |

The two measured peaks and semantic hashes remain unchanged. Retry's live
allocation is still 93,464 B before recovery and 93,328 B after it. Code-size
changes differ between these separately linked workload images; no inferred
single delta replaces the two measurements. Both gates remain unchanged and
are met. The increased Retry code use is recorded, not attributed to the
future Node layout.

## Checks executed at the A-T4-4 checkpoint

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`:
  passed.
- `cargo doc --locked --workspace --no-deps`: passed, no warnings.
- `cargo test --locked --workspace --all-features --no-fail-fast`, with
  `RASTOK_SAVE_GOLDENS` unset: **1,576 passed, 0 failed, 0 ignored, 61 groups**.
- `cargo check --locked -p rastok-core --no-default-features` for
  `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and
  `thumbv8m.main-none-eabihf`: all passed.
- RustRover MCP `build_project` after the admission changes: successful,
  no reported problems.
- Rustdoc citation lint: 211 Rust files, no relative Markdown links;
  the lint's four own tests passed.
- Both normal Cortex-M33 images and both `node-layout-probe` images built
  with `-D warnings` and completed in QEMU. The combined probe also passes
  the spike's release `cargo check` with both features enabled.

The slow host test-binary launches did not lead to changed timeouts or
security settings. Embedded builds resumed only after the network test
groups had finished. No golden-saving mode was enabled. No package, trace,
recording, fixture or example changed. The human-review and vocabulary
ledgers and `decisions.md` are untouched. There is no push, tag or publication.

**Lot 1 remains open:** the single production Node table, its private derived
routing indexes, indexed mailbox deliveries, package cutover and differential
qualification are not delivered by this checkpoint. The experiments establish
what to measure next; they do not substitute for those changes.

## Execution interface naming checkpoint

On 13 October, Yoann adds the execution-interface vocabulary to the rename
lot. `Vm` and `runtime::vm` become `Execution` and `runtime::execution`,
including the module files, fault and inspection types. `accept_input` and
`accept_connector_output` are alternative admission entry points. The
internal sequence is prepare, journal commit, then apply to live state;
outbound Call results still settle through Driver polling. The comments
describe these responsibilities without changing admission or recovery.

The existing executable-table index is now `NodeIndex`. This is only its
name: the owner and executor tables have not yet been merged, `OwnerSlot`
still exists, and handlers remain in their existing table. The package
format remains 20. There is no alias for the old public names.

RustRover performed the symbol renames. Verified references missed by the
IDE, including the spike outside the workspace, were updated separately;
module and test files were moved to match their names. The two human-review
registers receive only the naming addition authorized by Yoann; their
earlier entries and `decisions.md` are unchanged.

Checks after this naming addition:

- Formatting, all-target/all-feature strict Clippy and workspace rustdoc:
  passed, with no reported warning.
- The `execution`, `contracts`, `gauge_fixture`, `jsonl_fixture`,
  `plan_package` and `golden_trace` integration suites: **207 passed,
  0 failed, 0 ignored**. Golden saving was disabled.
- The three `no_std` targets listed above: passed.
- RustRover build: successful, no reported problems.
- Rustdoc citation lint: 213 Rust files, no relative Markdown links;
  all four tests of the lint passed.
- No package, trace, recording, fixture or example changed. Echo, JSONL
  and gauge retain their checked package, semantic and causal witnesses.

The complete workspace suite and the embedded execution/size measurements
were **not rerun for this naming checkpoint**. Their preceding results
above describe the A-T4-4 checkpoint, not new measurements. They remain
required before qualifying the production Node representation. Cargo
checks are run serially with the IDE build to avoid contention on `target`.

## Execution methods and Operator containment — 13 October

`9218779` finishes the separately committed interface naming: `new*`,
`advance_to`, `logical_time` and `has_ready_work`. The initial full-workspace
test command for that naming snapshot was interrupted without a reported
failure, after the `contracts` suite, so that the next checks exercise the
current structural work instead. It is **not** a completed workspace gate.

The next lot 1 step puts `vars` and `handlers` in `NodeExecution::Operator`.
The common Node no longer has DSL state, the plan no longer owns a separate
handler vector, and `Handler` no longer stores its owner. Iteration derives
the owner from the containing Node. The existing executor ordering and the
canonical activation ordering are preserved, including the flat indices
used for validation diagnostics and derived handler context.

The format-20 codec still writes the same tables and owner numbers. It
derives them from containment when encoding and checks the target Operator
when decoding. This step introduces no additional package reader, alias or
format bump. The next package number remains for the complete Node and
wiring cutover. No identity recipe changes.

Two hostile-plan tests move to the package boundary: a handler cannot name
a Connector or an out-of-range owner, states the Rust representation can no
longer construct. Additional package tests cover forged Connector DSL state
and a reordered cross-Operator handler table: decoding must reject it, not
silently canonicalize it. `extensions.md` §2 already excludes Connector DSL
state; the old common `Node::vars` could represent it and its validator had
no explicit Connector-state rejection. The new decoder refuses that forged
shape rather than discarding its state.

Operator work selection now borrows its handler slice instead of collecting
a temporary filtered vector. The optional layout probe follows the new
control representation, so it does not count the contained handlers twice.
Neither this change nor the previous copied-layout probes selects the final
physical layout of the unified Node table.

The two normal Cortex-M33 workload images were rebuilt with `-D warnings`
and executed separately, without a concurrent network test run. They decode
the same unchanged packages as the earlier measurements.

| Operator-contained representation | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap after package decoding | 6,824 B | 5,620 B |
| Decoder peak | 35,496 B | 30,352 B |
| Whole-run allocator peak | 112,116 B | 130,528 B |
| Heap change from the A-T4-4 checkpoint | −540 B | −32 B |
| Heap margin below 128 KiB | 18,956 B | 544 B |
| `.text` | 902,484 B | 962,716 B |
| `.rodata` | 70,340 B | 81,104 B |
| Code total | 972,824 B | 1,043,820 B |
| Code change from the A-T4-4 checkpoint | +644 B | +1,756 B |
| Code margin below 1 MiB | 75,752 B | 4,756 B |
| Decode ticks, instruction-count proxy | 21,807 | 16,048 |
| Journal records | 67 | 17 |
| QEMU result | `SPIKE_OK` | `SPIKE_OK` |

Retry's live heap is 93,432 B after execution and 93,296 B after recovery.
Both semantic hashes remain those recorded above. The two gates are met,
but Retry's remaining code margin is only 4,756 B; this is not spare space
to allocate to the future unified table without a new measurement.

Checks for this structural checkpoint:

- Workspace formatting, strict all-target/all-feature Clippy and workspace
  rustdoc: passed, no reported warning. Clippy was rerun after correcting
  the two new test programs.
- Both library test suites and the fifteen integration suites below:
  **1,257 passed, 0 remaining failures, 0 ignored**. The first run found
  two invalid new test programs, before package forging: a Process lacked
  its required `behavior`, and a graph had no input type constraint. After
  supplying those declarations, all 27 `plan_package` tests passed; no
  assertion or identity pin was changed to resolve either failure.
- The three `no_std` targets listed above: passed.
- RustRover MCP `build_project` after the final edits: successful, no
  reported problems.
- Rustdoc citation lint: 213 Rust files, no relative Markdown links;
  its four tests passed.
- Both normal Cortex-M33 images completed as measured above. The optional
  layout probe compiles with `node-layout-probe,retry-qualification`; its
  allocation comparison was not rerun for this checkpoint.
- `git diff --check` and standalone formatting of the edited
  `node_layout.rs`: passed. An additional whole-spike formatting check
  reports import/module ordering in `main.rs`, unchanged by this structural
  slice; that unrelated formatting is not included here.
- No package, trace, recording, fixture or example changed. The checked
  echo, JSONL and gauge witnesses passed without saving new references.

The selected test command, with golden saving disabled, was:

```sh
env -u RASTOK_SAVE_GOLDENS cargo test --locked --workspace --all-features \
  --lib --test plan_validation --test plan_package --test execution \
  --test contracts --test literals --test member_presence --test multiport \
  --test gauge_fixture --test jsonl_fixture --test golden_trace \
  --test v16_loop --test v16_loop_composition --test v16_recovery \
  --test retry_runtime --test instance_stop --no-fail-fast
env -u RASTOK_SAVE_GOLDENS cargo test --locked -p rastok --all-features \
  --test plan_package
```

This is a **targeted qualification, not a complete workspace test run**.
The earlier complete-suite result does not describe this representation.
Lot 1 remains open: one Node table, one reference wiring per Graph, private
derived routing indexes, indexed mailbox deliveries, the package cutover
and the final differential/workspace gate still remain. No push, tag or
publication is performed.

## Root Graph port ownership — 14 October

The root App's declarations now reside in its Graph owner, exactly like
the declarations of nested Graphs. `ExecutionPlan` no longer carries a
separate `boundary` vector; `boundary()` borrows the root's ports and
`graph_ports()` has no root-specific branch. Executor ports still belong to
their executable Node rows until the two declaration tables are merged.

The format-20 codec preserves its current wire layout: the root contract
is written at plan key 2, while root-owner key 6 stays empty. Decoding moves
that contract into the root Graph. A forged second root contract is rejected
before that move, with the existing `InvalidGraphContract` diagnostic;
nothing is overwritten or silently accepted. The serialized-depth walk
continues to count root types at their actual wire location, and root
annotations continue to use their resolved types. There is no new package
reader, format bump or change to semantic hashing.

A regression first failed on `5513883` because the root Graph's own port
table was empty. It now requires both the root and a nested Graph to own
their declared typed ports. A separate hostile-package test supplies both
root-contract locations and requires rejection. Existing package round trips,
depth checks, annotation checks and identity witnesses remain unchanged.

The optional layout probe reads the root declarations from their new owner
and no longer copies a separate boundary vector in its control sample.
This step does not choose the final physical layout of the single Node table.

The complete workspace test run now covers both this step and the preceding
Operator-containment step: **1,580 passed, 0 failed, 0 ignored, 61 groups**.
It ran with golden saving disabled:

```sh
env -u RASTOK_SAVE_GOLDENS cargo test --locked --workspace --all-features \
  --no-fail-fast -- --quiet
```

The host again took substantial time to start test executables. An additional
`--list` invocation prepared their startup in parallel; it ran no tests and
does not contribute to the count above. The full Cargo run completed normally,
including the real local-Driver and CLI tests. No timeout or system security
setting changed, and no build ran alongside the network tests.

Workspace formatting, strict all-target/all-feature Clippy and rustdoc pass.
The three `no_std` checks (`thumbv7m-none-eabi`,
`riscv32imac-unknown-none-elf`, `thumbv8m.main-none-eabihf`) pass as well.
RustRover MCP's final build reports success with no problems. The rustdoc
citation lint checks 213 Rust files with no relative Markdown links, and
its four tests pass.

The two normal Cortex-M33 images, built with `-D warnings` and without the
layout probe, retain their packages, semantic hashes and record counts.

| Root-owned App ports | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap after package decoding | 6,824 B | 5,620 B |
| Decoder peak | 35,496 B | 30,352 B |
| Whole-run allocator peak | 112,116 B | 130,528 B |
| Heap change from `5513883` | 0 B | 0 B |
| Heap margin below 128 KiB | 18,956 B | 544 B |
| `.text` | 902,668 B | 963,028 B |
| `.rodata` | 70,340 B | 81,104 B |
| Code total | 973,008 B | 1,044,132 B |
| Code change from `5513883` | +184 B | +312 B |
| Code margin below 1 MiB | 75,568 B | 4,444 B |
| Decode ticks, instruction-count proxy | 21,806 | 16,048 |
| Journal records | 67 | 17 |
| QEMU result | `SPIKE_OK` | `SPIKE_OK` |

Retry's live heap remains 93,432 B after execution and 93,296 B after
recovery. Both gates hold without a change to either limit. The remaining
544 B of heap and 4,444 B of code do not qualify the future unified layout;
that representation still needs its own complete measurements.

Both updated `node-layout-probe` images also compile with `-D warnings` and
finish in `SPIKE_OK`. Their equality checks include the root's moved port
declarations. The control now copies two declaration vectors instead of a
third, separate boundary vector; its measured stack-resident vector headers
total 24 B. These are still isolated copies, not executions of a unified plan.

| Current copied declarations, allocator bytes | Echo | Retry |
|---|---:|---:|
| Owner and executor tables | 3,216 | 2,544 |
| Single Node table, inline bodies | 3,192 | 2,580 |
| Single Node table, boxed bodies | 3,044 | 2,504 |
| Single Node table, compact side tables | 3,044 | 2,504 |

| Current declarations + wiring + indexes, resident / construction peak | Echo | Retry |
|---|---:|---:|
| Owner and executor tables | 3,584 / 3,592 B | 2,816 / 2,824 B |
| Inline bodies | 4,084 / 4,892 B | 3,268 / 3,984 B |
| Boxed bodies | 3,936 / 4,744 B | 3,192 / 3,908 B |
| Compact side tables | 3,936 / 4,744 B | 3,192 / 3,908 B |

The combined boxed experiment is therefore 352 B above the current echo
control and 376 B above the Retry control. It still must be measured with
the production decoder, execution and recovery before selecting the layout.
Its instrumented code size is not the normal-image code-gate measurement.

Final diff checks and formatting of the edited spike module pass. No
package, causal trace, recording, fixture or example changed; no reference
was regenerated. The human-review ledgers and `decisions.md` are untouched.
Lot 1 remains open for the single Node table, private derived routing indexes,
indexed mailbox deliveries, package cutover and differential qualification.
No push, tag or publication is performed.

## Shared validated scope chains — 14 October

This step implements the scope-chain part of the private derived indexes
in D-2026-10-11-1. `ValidatedExecutionPlan` derives each complete path from
App through the containing Graphs to its owner, after structural validation
has proved the parent tree. Live execution and recovery borrow the same
read-only index. No scope is skipped and no dynamic instance generation is
stored there. Admission, direct inbound Call responses, Graph Loops and
Closing use the same chains and order as before.

The former per-Execution `Vec<Option<Vec<OwnerSlot>>>` disappears. A missing
chain is no longer an internal state of a validated owner. The derived
index is private, absent from the raw plan and package, and does not enter
any journal field or identity recipe. RustRover performed the scope-chain
symbol renames; structural edits moved the derivation into validation.

Two focused tests cover complete nested chains and sibling separation,
package round-trip reconstruction, out-of-range queries, and rejection of
self, forward and invalid parents before index construction. Existing
instance, Graph Loop, Closing and recovery tests remain unchanged.

### Measured representation choice

Two implementations were compared on the normal Cortex-M33 images, against
`dabda8e`. The first stored all chain elements in one boxed slice plus an
offset slice. It reduced the Retry peak by 32 B, but added 1,224 B of code.
The retained implementation uses a `Vec` of chains, each allocated at its
exact validated path length. It reduces the Retry peak by 16 B and code by
144 B. Packing would save only another 16 B of heap while costing 1,368 B
of code relative to this simpler representation. It is not retained.
This comparison does **not** select the future unified Node-body layout.

The same unchanged echo package and temporary three-attempt Retry package
were executed with `-D warnings`, no probe instrumentation, and the existing
QEMU instruction-count configuration. The Retry workload still has one
operation, 16 frames per channel and 4 bytes per frame, with full recovery.

| Normal image | Echo, 32 admissions | Retry, 3 attempts + recovery |
|---|---:|---:|
| Heap after package decoding | 6,884 B | 5,660 B |
| Decoder peak | 35,556 B | 30,392 B |
| Whole-run allocator peak | 112,092 B | 130,512 B |
| Heap change from `dabda8e` | −24 B | −16 B |
| Heap margin below 128 KiB | 18,980 B | 560 B |
| `.text` | 902,628 B | 962,876 B |
| `.rodata` | 70,340 B | 81,112 B |
| Code total | 972,968 B | 1,043,988 B |
| Code change from `dabda8e` | −40 B | −144 B |
| Code margin below 1 MiB | 75,608 B | 4,588 B |
| Decode ticks, instruction-count proxy | 21,827 | 16,067 |
| Journal records | 67 | 17 |
| QEMU result | `SPIKE_OK` | `SPIKE_OK` |

The index now exists at package load, so resident plan and decoder peak
grow by 60 B for echo and 40 B for Retry. Those allocations replace, rather
than supplement, the per-Execution cache; the whole-run peaks fall as shown.
Retry's live heap is 93,416 B, and its recovered heap is 93,280 B. Both gates
hold without changing either limit. The semantic hashes remain
`562396de22b068ef874b8d7273bf972ee1b08b8dcb6dab8902d6b816b1c90813`
and `d8e89d6dc345e7c6199ee5ffc5664b682e09040a8589c6e934016bbe1c32581f`.

Both updated `node-layout-probe` images also build with `-D warnings` and
finish in `SPIKE_OK`. All candidates and the control now use the same exact
chain allocation policy. Their declaration-only measurements are unchanged.

| Declarations + wiring + indexes, resident / construction peak | Echo | Retry |
|---|---:|---:|
| Owner and executor tables | 3,560 / 3,560 B | 2,800 / 2,800 B |
| Inline bodies | 4,060 / 4,892 B | 3,252 / 3,984 B |
| Boxed bodies | 3,912 / 4,744 B | 3,176 / 3,908 B |
| Compact side tables | 3,912 / 4,744 B | 3,176 / 3,908 B |

These remain allocation experiments, not executions of a unified plan. In
particular, the boxed candidate still costs 352 B more than the current echo
control and 376 B more than the Retry control; updating the shared index
does not remove the need for complete decoder/execution/recovery qualification.

### Verification

Workspace formatting, strict all-target/all-feature Clippy, rustdoc without
warnings, and the three `no_std` checks pass (`thumbv7m-none-eabi`,
`riscv32imac-unknown-none-elf`, `thumbv8m.main-none-eabihf`). RustRover's
project build succeeds and its inspections find no error in validation or
Execution. The rustdoc-link check covers 213 Rust files with no relative
Markdown links; its four regression tests pass. The edited spike module
passes its standalone formatting check.

The final complete workspace test run passes: **1,582 tests, 61 groups,
zero failures and zero ignored tests**, two tests more than `dabda8e`.
The initial run had 1,580 passes and the two failures described below.
No code changed between the two runs and no compilation overlapped the
network probes. Fresh host binaries again took minutes before entering
their test harnesses. A one-second
`/usr/bin/sample` of the waiting `agent_chat --list` process showed only
`_dyld_start`, with no Rust frame. This identifies the observed wait as
pre-harness startup, not its operating-system cause. Listing the 59 compiled
executables is only startup preparation, not test execution or a passing gate.
No test timeout or system protection is changed.

The initial run encountered one startup failure in the chat's local-network
test: its child CLI had not bound the listener within the unchanged 30 s
limit, with empty stdout/stderr. The same newly built CLI, started separately
with `--help`, was sampled at `_dyld_start` more than 30 s after launch, before
any graph or Rust handler ran. After the CLI reached its argument parser, the
same chat test passed in 1.25 s with no rebuild, source edit or timeout change.
The second complete run passes all four chat tests. The first failure is
not counted as a pass.

The same initial run also exceeded the existing 20 s conformance deadline
for Process's `DecodeFailed` case, whose external command is
`perl -e 'print chr(255)'`. Repeating the unchanged full Process battery
passed in 0.42 s. There is no sample of that failed child, so its exact cause
is not established; the successful rerun does not erase the initial failure.
The second complete run passes all four Process conformance tests as well.

The final host and target checks were:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo doc --locked --workspace --no-deps
env -u RASTOK_SAVE_GOLDENS cargo test --locked --workspace --all-features --no-fail-fast -- --quiet
cargo check --locked -p rastok-core --no-default-features --target thumbv7m-none-eabi
cargo check --locked -p rastok-core --no-default-features --target riscv32imac-unknown-none-elf
cargo check --locked -p rastok-core --no-default-features --target thumbv8m.main-none-eabihf
node tools/ci/check-rustdoc-links.mjs
node --test tools/ci/check-rustdoc-links.test.mjs
git diff --check
```

No example, fixture, golden, package, trace or diagnostic recording changed.
The human-review registers and decisions are untouched. Lot 1 remains open:
the unified Node table, reference wiring, indexed mailbox deliveries and
the package cutover still need implementation and end-to-end qualification.

## Unified Node candidate — 14 October, not qualified

The working candidate moves every Graph, Operator and Connector into the
same Node table. The App Graph is the first row. Graph rows precede the
Executors; the relative Operator/Connector execution order remains unchanged.
Each row contains its path, parent, ports, Extensions, instance policy and
immutable declarations. Only an Operator body contains variables and handlers.
The separate owner table and reciprocal owner/executor references disappear.

The package reader now identifies this incomplete candidate as format 21;
no reference package, causal golden or diagnostic recording has been regenerated.
This is not an approval of the package or of the boxed body layout. Reference
wiring, indexed deliveries and complete host/embedded qualification still follow.

The unified declaration table receives new plan key **12**. It does not
reinterpret the former executor table at key 1 or the owner table at key 9.
Its Node-row map is a new record shape under that new key; existing package
keys, Extension tags and identity domains are not reassigned. The interim
physical-edge encoding now names the root Node explicitly, not `null`;
the final per-Graph wiring will replace transport of those derived edges.
No intermediate candidate representation is a published or approved format.

`NodeIndex::ROOT` names the root App. Every physical endpoint carries a
Node index. This removes the boundary-specific absence state from admission,
routing and recovery; the path-based text used in journal identities stays
unchanged. A package-unit test rejects a null endpoint and round-trips the
explicit root. The single declaration table also has one duplicate-path
check and one out-of-bounds error; reciprocal owner/Executor checks no longer
have a state to validate.

The package-unit handler encoding changes for a structural reason: handlers
are children of the Operator body, so their former executor reference is no
longer serialized. Its unit pin retains the activation and Dispatch bytes and
removes only the redundant owner entry. Hostile tests must reject a Graph body
carrying handlers. This package change does not affect a journal record, an
emission identity, the semantic hash, or a causal trace. The final package
witness inventory will be measured before any reference regeneration.

### Checkpoint verification

The declaration-table change and explicit root endpoints were compiled as
one batch with their host, runtime, recovery and test callers. No full test
gate was rerun between individual edits. The targeted batch has completed:

| Target | Tests passed |
|---|---:|
| Hosted library | 390 |
| `execution` | 85 |
| `golden_trace` | 8 |
| `plan_validation` | 74 |
| `retry_runtime` | 25 |
| `v16_loop` | 37 |
| `v16_loop_composition` | 5 |
| `v16_recovery` | 14 |

The core library batch ran 374 tests: 373 passed, and one hostile-codec test
failed while constructing its forged Operator handler, before calling the
decoder. Its base fixture has no handler; the mutation now inserts a complete
invalid handler rather than indexing an empty array. The targeted codec rerun
passed all 28 tests, including that rejection. Compilation without default
features passed on `thumbv7m-none-eabi`,
`riscv32imac-unknown-none-elf` and `thumbv8m.main-none-eabihf`.
Clippy found a single-element loop in a rewritten hostile handler test,
unnecessary borrows in the debugger projection, a redundant test binding and
one test still checking an absent boundary Node. Those and the test's missing
Node-index qualification have been corrected; strict Clippy passed on all
workspace targets and features. Rustdoc completed without warnings. The complete
workspace test gate and final reference-wiring qualification are not claimed.

The earlier declaration-only checkpoint ran 73/74 plan-validation tests and
4/5 Graph-composition tests. Their failures were assertions about the removed
duplicate owner table and an empty executor table, respectively; both now
check the same invariant against the unified Nodes and pass above. An
`instance_stop` package-version pin still expects 20 and is intentionally
unchanged until the final package inventory is approved. It is not a causal
trace failure, and no test has been ignored.

Candidate packages for the embedded measurements are generated outside the
repository, without replacing tracked witnesses. On this host a sample of
`retry_spike_package` more than eight minutes after launch found only
`_dyld_start`, before any Rust code, with a 96 KiB physical footprint. That
startup delay is not a Rastok benchmark. The generators subsequently completed
and the embedded results below were measured. Neither system security settings
nor test timeouts were changed.

### Candidate execution and memory measurements

Both Cortex-M33 workloads ran the unified Node candidate and ended with
`SPIKE_OK`, with the original semantic hashes and 67/17 journal records.
The package was generated from source into a fresh temporary directory and
selected through `RASTOK_SPIKE_PACKAGE`; no tracked package was replaced.

| Cortex-M33 measurement | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap immediately after package decoding | 6,784 B | 5,144 B |
| Package-load peak | 33,972 B | 29,120 B |
| Complete execution/recovery peak | 111,960 B | 128,940 B |
| Margin below 128 KiB | 19,112 B | 2,132 B |
| Live heap after execution | not sampled | 92,900 B |
| Live heap after recovery | not sampled | 92,764 B |
| `.text` | 902,916 B | 964,452 B |
| `.rodata` | 69,816 B | 80,556 B |
| `.text + .rodata` | 972,732 B | 1,045,008 B |
| Margin below 1 MiB | 75,844 B | 3,568 B |

Against `0f6cac3`, Retry's whole-run peak falls by 1,572 B and its resident
decoded plan by 516 B, while code grows by 1,020 B. Echo's whole-run peak falls
by 132 B and code by 236 B. These are measurements of the current Node-table
candidate, **not** of the remaining per-Graph wiring and mailbox-index work.
The gates remain unchanged, and this does not yet select the complete layout.

The package-ledger audit ran all 28 existing gallery scenarios. Each old/new
record count and causal seal matches. HTTP cases used only the documented
synthetic secret canaries and `fixture-v1` versions, with Lab responses; no
external model or partner was called. An initial run without these explicit
environment bindings stopped at the webhook's secret-version preflight and
is not counted as a successful gallery run.

Echo, JSONL and gauge also retain their semantic hashes. Their candidate
body lengths are 919, 964 and 225 bytes, respectively, versus 961, 1,005 and
249 bytes in the tracked format-20 packages. The body changes remove duplicate
declaration ownership and handler references and name the App endpoint
explicitly. These intermediate lengths are evidence, not approved fixture
pins: the final wiring shape must enter the package inventory first.

## Reference wiring candidate — 14 October

Each Graph now contains its reference `wiring`, normalized to one rule per
source. The compiler leaves the HIR used by semantic hashing unchanged.
The candidate package carries that wiring in the Graph body; it no longer
transports physical edges, composed selections, or a separate boundary
inventory. The reader rejects those extra tables rather than accepting a
second description of the same graph.

Validation derives private edge and selection indexes from the declarations,
including when revalidating a modified clone. Node paths, canonical order
and parent relationships are checked before resolving route endpoints.
Each Graph supplies the scope and boundary face of its wiring. Destination
coverage, crossed boundary types and manager-command authority remain
independently checked. Comparing three supplied routing inventories is no
longer a validation obligation; `PhysicalEdgeMismatch` disappears.

The runtime, annotation propagation, Loop proofs and debugger projection now
read the per-Graph declarations instead of a conditional boundary inventory.
The mailbox and journal representation do not change in this step.

This remains an **incomplete format-21 candidate**. Reference endpoints
still use the shared path-based routing vocabulary; converting those
endpoints to Node indices and mailbox deliveries to edge indices remains
part of lot 1. No final layout, package witness, or complete lot is approved
here. The human-review ledgers and `decisions.md` are unchanged.

### Checks in this batch

- Workspace/all-feature/all-target compilation and the RustRover build
  passed. Strict all-target/all-feature Clippy passed without warnings.
- The first combined run exercised 1,204 tests: 1,199 passed, five failed.
  Three failures came from test construction: two tests still selected
  Node 1 for the App boundary, and the new cache-rebuild regression removed
  an inferred output's only type constraint. A fourth was the new hostile
  endpoint test expecting the wrong exact error text for a non-array owner.
  Those fixtures are corrected without weakening their behavioral assertions.
  The fifth is the unchanged `FORMAT_VERSION == 20` package pin; it remains
  pending the final format-21 witness inventory. Rerun results follow below.
- The 16 JavaScript graph-layout tests passed. The browser test did not
  start: the available Node environment cannot resolve `playwright`.
  No dependency was installed and no browser pass is claimed.
- No package, trace, recording, example or conformance fixture was changed.

Host startup time is not reported as test or runtime performance. Sampling
the core test binary before its first test found it at `_dyld_start`, with
a 112 KiB process footprint and no Rust frame. No timeout or system-security
setting was changed to avoid that startup delay.

The focused rerun passed all 85 Contract tests, 63 multiport tests and 76
plan-validation tests. The instance-stop suite passed 20 tests; its remaining
failure is the same unchanged format-20 assertion, after its forged lexical
scope has already been rejected correctly. All 28 core codec tests passed on
rerun, including rejection of each removed top-level routing table. Together
with the earlier batch this covers 1,225 distinct tests: 1,223 have a passing
result and two package-version assertions remain pending. No test is ignored;
the full workspace test gate is not claimed.

Formatting, rustdoc without warnings and compilation without default features
passed for `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and
`thumbv8m.main-none-eabihf`. A final RustRover build also passed.

### Trace and embedded qualification

All 28 gallery scenarios again passed with the same record counts and causal
seals as the tracked traces. Echo, JSONL and gauge keep their semantic hashes.
The HTTP scenarios used only explicit synthetic canaries and Lab responses;
no model or partner was called. Candidate packages were generated from source
in a temporary directory, not written over tracked witnesses.

| Witness | Tracked format-20 body | Candidate body | Candidate checksum |
|---|---:|---:|---|
| Echo | 961 B | 980 B | `0b09b2234601e586127b8631ec37a11f1fffbe8656e676edfaa1466750a3579c` |
| JSONL | 1,005 B | 1,023 B | `2ecab68fa19ab6f5a8b8401382ca2032cf14b907291efee38ec9c073ce939726` |
| Gauge | 249 B | 248 B | `cc6e03ca15d0fc2c32d94f897e13818e77e5e7356c0bb620e1e56b1ebdec282f` |

The body now describes each Graph's wiring instead of independently supplied
physical and boundary tables. These are intermediate measurements, not final
package pins: endpoints are still path-based in this checkpoint.

Both Cortex-M33 workloads ended with `SPIKE_OK`, unchanged semantic hashes
and 67/17 journal records respectively.

| Cortex-M33 measurement | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap immediately after package decoding | 8,460 B | 6,220 B |
| Package-load peak | 37,836 B | 31,768 B |
| Complete execution/recovery peak | 113,636 B | 130,016 B |
| Margin below 128 KiB | 17,436 B | 1,056 B |
| Live heap after execution | not sampled | 93,976 B |
| Live heap after recovery | not sampled | 93,840 B |
| `.text` | 898,820 B | 960,428 B |
| `.rodata` | 69,828 B | 80,528 B |
| `.text + .rodata` | 968,648 B | 1,040,956 B |
| Margin below 1 MiB | 79,928 B | 7,620 B |

Against `7446583`, the Retry heap peak grows by 1,076 B and code decreases
by 4,052 B; Echo grows by 1,676 B and code decreases by 4,084 B. The limits
and workload are unchanged. The remaining 1,056 B of Retry heap is not an
allowance for unmeasured growth. Indexed reference endpoints and mailbox
deliveries still require qualification before the final package approval.

The optional layout probe now compares complete declarations, wiring and
execution indexes only. Its earlier declaration-only comparison would count
the current Graph's wiring against empty experimental Graph bodies and is
therefore removed rather than reported as a layout saving.
The updated probe compiled with `retry-qualification,node-layout-probe`;
its Rust formatting check passed. This checkpoint does not claim a new
allocation-probe run.

### Browser rerun — 14 October

The earlier `MODULE_NOT_FOUND` was a module-resolution failure, not evidence
that Playwright was absent. Node does not automatically search global module
installations, and `RASTOK_PLAYWRIGHT` was unset in that first command.
The existing local installation was used explicitly; no package or browser
was installed and no repository dependency was added.

After `cargo build --locked -p rastok --features hosted-ui,hosted-websocket`,
the full `browser.test.cjs` suite passed **35 tests, zero failures, zero
skipped**, in 61.7 seconds against `271de9d`. The run used Playwright **1.62.1**
from the local Codex runtime and the installed Google Chrome executable,
selected through `RASTOK_PLAYWRIGHT` and `RASTOK_CHROME`. This is local-browser
qualification, not a claim that the CI-pinned Playwright 1.58.2 environment
or the independent reviewer gate has run.

The cases cover graph and mailbox inspection, original App emissions,
causality, Loop/Retry state, LIVE/HISTORY/REPLAY navigation, keyboard use,
secret masking, and recording/replay of a local `cat` execution. HTTP and
agent scenarios used Lab responses and explicit synthetic canaries; they
did not invoke an external model or partner. No timeout, assertion, trace,
recording fixture, package pin or application code changed for this rerun.

## Indexed reference endpoints — 14 October

The candidate plan now uses one `Endpoint { node: NodeIndex, port }` for
reference wiring and derived physical edges. `Destination` distinguishes
a port, explicit discard, a Loop command and an exact-instance closure.
Command owners and closure scopes refer to the same Node table. The Graph
containing a rule still determines the face of each boundary port.

The validator checks every referenced index before composing routes or
indexing the Node table. A hostile-plan regression covers source ports,
destination ports, Loop owners, closure scopes and closure targets. Port
direction, containment and command authority remain separate checks; an
in-range number is not proof that a destination is allowed.

The source HIR keeps its path-based types and semantic-hash writer. Lowering
resolves those paths only after constructing the complete Node index, so a
Graph can name children emitted later in the table. The checker uses a
temporary index to reuse the core composition proof, and translates its
diagnostics back to source paths. No compatibility alias is introduced.

Node indices are not causal identities or scheduling order. Runtime route
selection preserves the existing path order, including Executor destinations
before Graph boundaries. The instance-stop journal fact now owns its source
port and Graph/Executor distinction directly; its exact generation already
carries the source path. Its encoding and identity recipe are unchanged and
contain no Node index. An independent construction test checks both source
forms against the existing canonical tuple and identity recipe.

This is still the same format-21 candidate, not another format bump or a
package approval. Indexed mailbox deliveries and the final package witness
inventory remain to be completed. No trace, recording, golden, conformance
fixture, example or human-review ledger is changed by this step.

### Trace and embedded results

The 28 gallery scenarios pass, with identical old/new record counts and
causal seals. Echo, JSONL, gauge and the Retry spike also keep their semantic
hashes. The first gallery invocation omitted the synthetic secret-version
variables and stopped at attestation; the successful rerun supplied both
fixture tokens and their `fixture-v1` versions. All HTTP results came from
the Lab; no external model or partner was called.

Candidate packages were regenerated only in a temporary directory. These
are measurements for the final inventory, not approved package witnesses:

| Witness | Tracked format-20 body | Indexed candidate body | Candidate checksum |
|---|---:|---:|---|
| Echo | 961 B | 934 B | `c24806e6918130b4d8d35c6660d3f254c598d17c41aac9930f6545d4cf748dc0` |
| JSONL | 1,005 B | 979 B | `6d03694522f95d6ea2b8cf3654938ee73f65e5fa75c6dae8bcfc83943ed733d6` |
| Gauge | 249 B | 230 B | `3acf7c7f98b481b4d779f99a234ccea6173136e6e2a79417da99164f968b0196` |

Both normal Cortex-M33 workloads finish with `SPIKE_OK`. Neither the
128 KiB heap gate, the 1 MiB code gate nor the workload has been relaxed.
The optional layout-probe feature is absent from these measurements.

| Cortex-M33 measurement | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap immediately after package decoding | 7,692 B | 5,708 B |
| Package-load peak | 36,160 B | 30,684 B |
| Complete execution/recovery peak | 112,868 B | 129,504 B |
| Margin below 128 KiB | 18,204 B | 1,568 B |
| Live heap after execution | not sampled | 93,464 B |
| Live heap after recovery | not sampled | 93,328 B |
| `.text` | 898,900 B | 959,732 B |
| `.rodata` | 70,220 B | 80,928 B |
| `.text + .rodata` | 969,120 B | 1,040,660 B |
| Margin below 1 MiB | 79,456 B | 7,916 B |
| Journal records | 67 | 17 |

Against the preceding reference-wiring checkpoint, indexed endpoints reduce
the complete heap peak by 768 B for echo and 512 B for Retry. Code grows by
472 B for echo and shrinks by 296 B for Retry. These are separate linked
images, not an assumed common delta. The remaining Retry margin still
requires measuring the mailbox-delivery step before accepting its layout.

### Verification of the indexed checkpoint

- Workspace test compilation with all features passed. Strict Clippy for
  all workspace targets and features passed after removing one unnecessary
  reference. Workspace formatting and rustdoc passed without warnings.
- The three `no_std` checks passed: `thumbv7m-none-eabi`,
  `riscv32imac-unknown-none-elf`, `thumbv8m.main-none-eabihf`.
- RustRover's final build succeeded with no reported problem. Its semantic
  renames handled `EdgeEnd` and `RouteDestination`; structural conversions
  and the verified out-of-workspace probe references use ordinary patches.
- The modified layout probe passes release compilation with
  `retry-qualification,node-layout-probe` and `-D warnings`, and its own
  Rust formatting check. No new allocation-probe run is claimed.
- The citation lint checked 213 Rust files with no relative Markdown links;
  all four tests of the lint passed.
- All 16 graph-layout JavaScript tests and all 35 browser tests passed,
  with no failure or skipped test. The browser run took 53.7 seconds using
  the existing local Playwright 1.62.1 and Chrome, not the CI-pinned 1.58.2
  environment. No browser dependency, timeout or assertion changed.
- The public CLI checks echo with the same semantic hash. No tracked
  package, recording, trace, fixture or example was overwritten, and no
  push, tag or publication was performed.

The focused all-features run covers 25 compiled test binaries: **1,303
passed, three failed, none ignored**. The three failures are the unchanged
format-20 assertions in `plan_fields`, `instance_stop` and `retry_plan`;
the candidate reports format 21. Their package-version assertions remain
unchanged until the final inventory and package approval. No behavioral
assertion failed. This is not a claim that the complete workspace suite is
green: the remaining package witnesses still require regeneration at the
end of the lot. After compiling every workspace test target with
`cargo test --locked --workspace --all-features --no-run`, the focused
binaries ran in four workers with `--test-threads=4`, without fixture-saving
variables. An earlier serial invocation was stopped and replaced by this
complete focused run; it is not counted a second time.

## Indexed mailbox deliveries — 14 October

The mailbox representation now stores `Delivery.edge: EdgeIndex`; the
arrival port and canonical cursor path live once in an execution-local
`DeliveryRoutes` table. Queue positions remain per route and exact source
generation. Registration happens when committed delivery work is applied,
and recovery reconstructs the table from the same recorded decisions.

This is not an index into the plan's spatial DAG. Graph boundary admissions
and Loop/Retry readmissions also need cursor paths, but are not physical
back-edges. The table shares those existing descriptions without adding an
edge, command, port or journal field. Every identity recipe and consumed
position still receives the original path text. A regression registers the
same route at different indices and compares the complete resulting records
and their canonical encodings. Other regressions cover shared routes with
independent source cursors and typed rejection of an absent caller-supplied
index.

`Loop.state` is now a direct lazy payload binding, not a fabricated delivery
with an `@loop.state` edge. Join inputs remain borrowed during preparation;
their metadata merge, materialization rules and consumed positions do not
change. The hosted mailbox projection resolves the route into a
`DeliveryInspection` before creating a snapshot, so the debugger retains
standalone text and ports rather than exposing execution-local indices.
The broader inspection-module separation remains lot 2.

Route-name lookup is linear in the number of registered routes. This small
table is execution state, not a durable-journal index or a new recovery
authority. Like the existing per-route sequence map, it is retained for
the life of the M1 execution. No long-history scalability claim is made.

### Embedded measurements

Both unchanged Cortex-M33 workloads finish with `SPIKE_OK`, the same
semantic hashes and 67/17 records. The input packages are the preceding
indexed-endpoint candidates: this mailbox step changes no package bytes.

| Cortex-M33 measurement | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap immediately after package decoding | 7,692 B | 5,708 B |
| Package-load peak | 36,160 B | 30,684 B |
| Complete execution/recovery peak | 111,404 B | 129,188 B |
| Margin below 128 KiB | 19,668 B | 1,884 B |
| Live heap after execution | not sampled | 93,252 B |
| Live heap after recovery | not sampled | 93,116 B |
| `.text` | 898,748 B | 959,884 B |
| `.rodata` | 70,264 B | 81,020 B |
| `.text + .rodata` | 969,012 B | 1,040,904 B |
| Margin below 1 MiB | 79,564 B | 7,672 B |

Against `59f1ddf`, the complete heap peak decreases by 1,464 B for echo and
316 B for Retry. Echo code shrinks by 108 B; Retry code grows by 244 B.
The workload, allocator instrumentation and both acceptance gates remain
unchanged. Final format-21 package inventory and approval remain separate.

### Verification of indexed deliveries

Workspace formatting, strict all-target/all-feature Clippy, compilation of
all workspace tests, and rustdoc pass. The three `no_std` checks pass for
`thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and
`thumbv8m.main-none-eabihf`. RustRover's project build reports success with
no problem. The rustdoc-link lint checks 214 Rust files without a relative
Markdown link; its four tests pass.

All 28 gallery scenarios retain their record counts and causal seals. The
three package candidates have exactly the body lengths and checksums listed
in the indexed-endpoint inventory above: this delivery representation does
not change their bytes. The HTTP scenarios use synthetic canaries and Lab
responses, never a real model or partner.

All 16 graph-layout tests and all 35 browser tests pass. The successful
browser run uses the already-installed Playwright **1.61.1** and Google
Chrome, takes 54.6 seconds, and has no skipped test. No dependency was
installed. An initial run with the global Playwright 1.49.0 passed 26 tests
and failed nine: five CLI startup timeouts and four string-evaluation errors
against the page's Content Security Policy. The complete rerun used the
newer installation after the CLI had reached its argument parser. No code,
timeout, assertion or security policy was changed. The initial failures are
not counted as passes, and their exact startup cause was not established.

While the focused Rust suites were starting, a one-second sample of
`loop_host_exit` showed only `_dyld_start`, a 112 KiB footprint and no Rust
frame. This observation distinguishes that wait from an executing test; it
does not establish the operating-system cause or qualify an unrun test.

The focused all-features run is complete: **1,306 passed, three failed,
none ignored, across 25 test binaries**. It includes all three new delivery
regressions, journal codec and prefix-recovery checks, Loop/Retry, joins,
Closing, Contracts and representation tests. The only failures are the
unchanged `FORMAT_VERSION == 20` assertions in `plan_fields`, `instance_stop`
and `retry_plan`; each reports actual 21 versus expected 20. No behavioral
assertion failed. This is a focused run, not the final complete workspace
gate. The independent echo recording-tuple pin was also rerun successfully;
it is already included in the library count, not an additional test.

No tracked example, fixture, golden, package, trace or recording changes.
The human-review ledgers and decisions are untouched. Lot 1 still needs the
layout review, the inventoried package regeneration and documentation
alignment, then a complete workspace run without the remaining version or
package-pin failures. Inspection-module separation and the hosted Worker
remain lot 2.

## Independent depth budgets — 14–15 October 2026

D-2026-10-14-1 fixes the format-21 reader bound at 256, with the root at
level zero. Expression structure and parser recursion remain bounded at
128; types remain bounded at 128 and Dispatch at 32. The source tests
exercise the last accepted addition chain and parenthesis tower, then a
Check refusal with depth 129. The independent raw-plan test combines an
expression at its own bound with a growing structured literal: serialized
level 255 reads back, and the first refusal names exactly level 256.

The combined raw-plan stress test needs an explicit 8 MiB test-thread stack
in an unoptimized host build; its first run exceeded the default 2 MiB test
stack. This allocation belongs only to that test harness. It does not change
an expression, type, transport or runtime bound, and no assertion is removed.
The optimized embedded decoder is measured separately below.

The optional `stack-qualification` feature of the non-shipping Cortex-M33
spike implements Fable's watermark method: paint unused word-aligned RAM
between `__sheap` and MSP minus 32 bytes, then scan for the lowest overwritten
word. Every unsafe access states its invariant. Interrupts are disabled
during painting; the image runs one core. Each section is repainted before
measurement. Static allocation and the heap arena are below the painted
range. The result includes the caller's `main` frame, which differs across
images; only differences within the same image measure a per-level cost.

| Instrumented release image | Complete package decode stack | Execution/recovery stack | Package-load heap peak | Complete heap peak |
|---|---:|---:|---:|---:|
| Echo, 32 admissions | 13,840 B | 17,144 B | 36,160 B | 111,404 B |
| Retry, three Calls and full recovery | 9,960 B | 18,236 B | 30,684 B | 129,188 B |
| Deepest accepted combined package, decode only | **79,168 B** | not executed | 107,328 B | 107,328 B |

The combined package has 2,193 bytes and expression depth 128. It is generated
by the exact-boundary property test, not by a different source approximation.
The `decode-only` feature exits after decoding and does not claim execution of
that program. All three images finish with `SPIKE_OK`. The two workload heap
peaks are identical with and without the probe.

Within each image, the raw CBOR reader grows by **60,960 B** from level 1 to
level 255: **240 B per level**. Level 256 refuses with
`NestingLimitExceeded`. This reproduces the frame measurement behind the
decision; the measured 79,168 B complete path replaces the earlier 100–120 KiB
estimate for this constructed boundary case. It is below the approximately
257 KiB stack space discussed in the review, but the run is on QEMU
`mps2-an505` with the spike's 4 MiB RAM map, not on a physical RP2350.
Neither the 128 KiB heap gate nor the 1 MiB code gate is raised.

To reproduce, set `RASTOK_DEPTH_PACKAGE` to a new path when running only
`what_the_type_accepts_the_format_reads_back`; existing files are not
overwritten. Build the spike with `RASTOK_SPIKE_PACKAGE` pointing at that
file and `--features decode-only`. Echo uses `stack-qualification`; Retry
uses `retry-qualification,stack-qualification` with the unchanged one-operation,
sixteen-frames-per-channel, four-bytes-per-frame environment. All builds use
the pinned toolchain, release optimization and `-D warnings`; execution uses
Cortex-M33 QEMU with `-icount shift=0`.

Normal images, without the stack feature, still meet both gates. Echo uses
969,012 B of `.text + .rodata`, unchanged. Retry uses 1,040,912 B, eight bytes
more than the preceding bound-128 image, leaving **7,664 B** of code margin.
Its heap margin stays **1,884 B**. These bounds change no existing package,
journal record, causal seal or semantic hash; no witness is regenerated for
this decision. The separately inventoried Node-table package movement is
recorded in the [identity inventory](node-realignment-identity.md).

## Lot 1 closure — 14 October 2026

### Scope and representation

The App root, nested Graphs, Operators and Connectors occupy one Node table.
Node bodies are boxed; only the Operator body contains variables and handlers.
One `NodeIndex` names a declaration. Each Graph owns its reference wiring,
with indexed `Endpoint` values and explicit `Destination` variants; the Graph
containing a rule determines the face of its boundary ports. Validation derives
private execution routes and scope indexes from that description.

Queued deliveries carry an `EdgeIndex`, not a copied route string or port.
`DeliveryRoutes` resolves the canonical cursor path and arrival port when
preparing a transaction or reconstructing work. These indexes are execution
state, never identity material. Journal records retain paths and exact instance
generations. The differential regression changes internal registration order
and compares complete records and codec bytes, not just displayed outputs.

A-T4-4 is complete: `admit_proposal` prepares without changing live state,
commits once, then applies the accepted admission without another fallible
branch. The five typed source/exchange cases share source-instance reservation.
Call and Session refusal tests preserve retry identities and prove that a
failed journal commit leaves no partially applied exchange.

### Approved package update

The [identity inventory](node-realignment-identity.md) preceded regeneration
in commit `54c2b3e`. The three existing exact package-witness tests regenerated
echo, JSONL and gauge from source. The `package_ledger` example compiled a fresh
binary echo for the spike: **1,015 bytes**, byte-identical to the decoded hex
witness. Body lengths and checksums exactly match the pre-update inventory.
Only these four `.rkp` files move. The three explicit version assertions now
expect 21, as does the deliberately noncanonical-header fixture in
`plan_package`. The public-status test follows the current format registry
instead of pinning a historical approval date. No functional assertion or
timeout is weakened.

All 28 gallery comparisons preserve record counts and causal seals. Echo,
JSONL and gauge semantic hashes remain fixed, as does the independent echo
recording-v4 tuple digest. No `.trace`, source example, behavioral fixture,
journal key or identity domain changes. Older packages refuse by version;
there is no compatibility reader or converter.

The package document now describes the actual Node/body/wiring fields and
derived indexes. The live specifications link to the current format registry;
README, whitepaper and the implementation map agree with it. UML views 05 and
06 were regenerated locally and inspected as images. The human-review
registers remain untouched. D-2026-10-14-1 records the depth arbitration;
older decisions remain unchanged.

### Verification before the depth arbitration

The complete run of
`cargo test --locked --workspace --all-features --no-fail-fast` finished with
**1,588 passed, three failed, none ignored, across 61 groups**. The failures are:

- `a_non_canonical_envelope_refuses`: its hand-written header still spelled
  version 20. The correction spells 21 for both the valid prefix and the
  deliberately nonminimal integer; the canonicality refusal is unchanged.
- `current_public_status_matches_the_package_and_contracts`: the whitepaper
  header and catalogue still named format 20. They now name 21 and the current
  inventory; specification headers link to the checked format registry.
- `the_depth_boundaries_are_exact`: containing handlers in Operator bodies
  adds **three CBOR levels**. The previous accepted 118-addition expression now
  reaches level 130 and refuses against `MAX_NESTING = 128`. This is not just a
  stale version pin: it reduced the accepted source set. The test was left
  unchanged at that checkpoint. D-2026-10-14-1 subsequently resolved this
  issue with independent bounds, not a representation-specific offset.

The focused rerun of `plan_package` and `site_content` is complete:
**39 passed, one failed, none ignored**. Both version/documentation corrections
passed; only `the_depth_boundaries_are_exact` still failed before arbitration.
This was not a second full workspace run. No failing or pending check is
counted as passed; the final qualification follows below.

Checks completed before applying D-2026-10-14-1:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`:
  passed.
- `cargo doc --locked --workspace --no-deps`: passed without warnings.
- `cargo check --locked -p rastok-core --no-default-features --target <target>`:
  passed on `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and
  `thumbv8m.main-none-eabihf`.
- JavaScript layout, editor, chat and rustdoc-link checker tests: **36 passed**,
  zero failures or skips. The citation lint checks 214 Rust files with no
  relative Markdown doc link.
- Browser interaction suite: **35 passed**, zero failures or skips, in 54.1 s,
  using the installed Playwright 1.61.1 and Google Chrome. No package install,
  new browser service or external model call was needed.
- All 28 gallery scenario comparisons: unchanged counts and seals, using
  synthetic Lab credentials and responses only.
- All 696 relative file links in the 27 changed Markdown documents resolve.
  `git diff --check` passes; the protected review ledgers and decisions remain
  untouched.

The host again delays some linked executables before their test harness
starts. A one-second sample of the `generic_measure --list` process shows
only `_dyld_start`, a 96 KiB footprint and no Rust frame. Additional `--list`
launches prepare startup but do not execute tests and are not included in any
passing-test count. The Cargo artifact listing reported every artifact fresh;
no compilation overlapped the network tests. No timeout or security policy
was changed to obtain the results.

### Embedded reruns before the depth arbitration

Both workloads were rebuilt from their freshly compiled format-21 packages,
then executed under Cortex-M33 QEMU with `-icount shift=0`. Network tests had
finished before these builds. The fixed workloads and limits are unchanged.

| Measurement | Echo, 32 admissions | Retry with recovery |
|---|---:|---:|
| Heap immediately after package decoding | 7,692 B | 5,708 B |
| Package-load peak | 36,160 B | 30,684 B |
| Complete execution/recovery peak | 111,404 B | 129,188 B |
| Margin below 128 KiB | 19,668 B | 1,884 B |
| Live heap after execution | not sampled | 93,252 B |
| Live heap after recovery | not sampled | 93,116 B |
| `.text` | 898,748 B | 959,884 B |
| `.rodata` | 70,264 B | 81,020 B |
| `.text + .rodata` | 969,012 B | 1,040,904 B |
| Margin below 1 MiB | 79,564 B | 7,672 B |
| Journal records | 67 | 17 |
| QEMU result | `SPIKE_OK` | `SPIKE_OK` |

The Retry workload remains one operation, three Calls, sixteen frames per
channel, four bytes per frame, including full journal recovery. These are
workload measurements, not universal bounds for valid programs. They reproduce
the preceding indexed-delivery measurements exactly; no threshold was raised.

### Final verification after D-2026-10-14-1 — 15 October

The current patch passes workspace formatting, strict all-target/all-feature
Clippy and rustdoc without warnings. The three core `no_std` checks pass on
`thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` and
`thumbv8m.main-none-eabihf`. All 375 core tests pass. The focused source-depth
and composed raw-plan boundary tests each pass without a global stack override;
the raw-plan test's own explicit thread stack is described above. Its emitted
2,193-byte package is byte-identical to the one measured by the spike.
The citation lint checks 214 Rust files with no relative Markdown doc link,
and all four tests of that lint pass. The 67 newly added relative Markdown
destinations resolve; the earlier full-link and browser passes remain dated
above and are not presented as new runs.

The complete `cargo test --locked --workspace --all-features --no-fail-fast`
run exits successfully: **1,591 passed, zero failed, zero ignored, across
61 groups**, including the empty rustdoc groups. No build overlapped its
network tests. Earlier focused runs are not added to these totals. The normal
embedded images and all three stack-probe images pass; the measurements above
include the final eight-byte Retry code increase. Lot 1 is closed.

Several host executables waited before Rust entry, with samples showing
only `_dyld_start`. RustRover's project build passed with no reported problem.
The completed strict Clippy and subsequent gate use the repository's pinned
1.97.1 toolchain; the final commands select its binaries directly instead of
the Homebrew Rustup shell launchers. Blocked earlier invocations were stopped
and are not counted as successful checks. No timeout, assertion, security
setting or installed toolchain was changed.

### Handoff

`032656a` separates the depth budgets and records the stack qualification;
`17981f2` regenerates only the inventoried format-21 package witnesses and
their version pins. This closure aligns the public model and records the
completed gate without another artifact movement.

This closure does not start lot 2, claim a new independent Fable review, or
close external release gates. Separating inspection types from Execution and
introducing the hosted Worker remain the next lot. No push, tag or publication
is performed.

Planning estimate requested by the 11 October brief, supplied on 14 October
(working days, not a reconstruction of time already spent):

| Lot | Estimated engineering effort | Calendar condition |
|---|---:|---|
| 0 — vocabulary | 2–3 days | Already reviewed; not reopened here |
| 1 — Node ontology and A-T4-4 | 6–9 days in total | The remaining depth qualification and closure were estimated at 1–2 days |
| 2 — inspection separation and hosted Worker | 3–5 days, plus one review day | Starts only after the lot-1 report and Yoann's instruction |

The 20 October checkpoint must choose whether lot 2 fits the 23 October
engineering freeze with a complete gate and review. Keeping the 26–30 October
external Rust review and the 5 November opening is an option only if that
evidence is ready; this report does not silently move a date or reduce a gate.

## Fable review follow-up — 15 October

Fable independently reproduced the complete 1,591-test gate and all three
stack/heap probes on `35d3072`. The review does not reopen lot 1. Its two
documentation omissions are corrected: the opening checklist and diagnostic
recording contract now link to the current package registry and the Node
realignment inventory. The public-content test checks both live headers.

A source-level regression exercises the Plan-phase depth guard separately
from the expression guard: `size([[...1...]]) + 1 + 1` with 119 nested lists
encodes and reads back; 120 lists refuse with `E-PLAN-DEPTH`, phase Plan,
`depth = 256`, while expression/type depths remain below 128. The diagnostic
now names a zero-based **level** and the exclusive upper bound, rather than
appearing to allow the refused value. Like the raw-plan stress test, the
unoptimized host probe uses an explicit 8 MiB test-thread stack; the initial
2 MiB harness overflowed before reaching its assertions. With that harness
bound explicit, the new regression first failed on the ambiguous sentence.
The content regression first failed on the stale opening-checklist header.
No production bound or artifact changes in this follow-up.

Executed after the correction: formatting check, strict workspace Clippy
(all targets and features), rustdoc, and the two affected integration suites
(`plan_package`: 29 passed; `site_content`: 12 passed). No failure or ignored
test. The full suite and the embedded probes were not rerun for this
diagnostic/documentation follow-up; the preceding full gate and Fable's
independent reproduction remain dated evidence, not a new run.

### Reproducing the normal-image code measurements

The earlier normal-image sizes (969,012 B echo, 1,040,912 B Retry) used the
repository checkout at `/Users/steelcoder/Documents/rastok/rastok`, the pinned
`1.97.1-aarch64-apple-darwin` toolchain, and the spike's release profile
(`opt-level = "z"`, LTO, one codegen unit). The Cargo fingerprints retain
`RUSTFLAGS = -D warnings -C link-arg=-Tlink.x`. No stack or layout feature was
enabled. The commands below reproduce that build configuration, with a fresh
temporary path for the same Retry package:

```sh
export PATH="/Users/steelcoder/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH"
qualification_dir=$(mktemp -d /tmp/rastok-lot1-normal.XXXXXX)
cargo run --locked -p rastok --example retry_spike_package -- "$qualification_dir/retry.rkp" 3
cd spikes/rp2350-emu
env -u RASTOK_SPIKE_PACKAGE -u RASTOK_RETRY_OPERATIONS \
  -u RASTOK_RETRY_FRAMES -u RASTOK_RETRY_FRAME_BYTES \
  RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' cargo build --locked --release
/opt/homebrew/opt/llvm@18/bin/llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
RASTOK_SPIKE_PACKAGE="$qualification_dir/retry.rkp" \
  RASTOK_RETRY_OPERATIONS=1 RASTOK_RETRY_FRAMES=16 RASTOK_RETRY_FRAME_BYTES=4 \
  RUSTFLAGS='-D warnings -C link-arg=-Tlink.x' \
  cargo build --locked --release --features retry-qualification
/opt/homebrew/opt/llvm@18/bin/llvm-size -A target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350
```

The gate is the sum of `.text` and `.rodata`, not the tool's `Total` line.
Fable's normal images were 184 B smaller for echo and 176 B smaller for
Retry. The cause of that difference has not been established; the report
keeps the original, larger measurements and does not claim binary-identical
builds. Both measurements satisfy the same gates.

### M2 capacity observation

The level-255 combined package peaks at **107,328 B of heap for decoding
alone**, leaving 23,744 B below the 128 KiB gate before any execution or
durable-journal index is loaded. Its 79,168 B stack measurement does not
establish a corresponding heap margin for M2. Journal-depth and index work
must measure peak heap as well as stack; the tested package is one boundary
case, not a maximum-size bound for every admissible package.
