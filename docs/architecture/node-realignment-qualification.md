# Node ontology realignment — qualification in progress

Status: 13 October 2026. This records the preparation for lot 1 of
[D-2026-10-11-1](../spec/decisions.md#d-2026-10-11-1--realign-the-plan-and-the-runtime-on-the-specifications-node-ontology-before-the-opening).
The vocabulary follow-up is `2259fd4` and `beabf68`. The executable plan is
still format 20, with its existing owner, executable-node and handler tables.
The production plan representation is unchanged. Isolated allocation probes
compare possible Node bodies; no new physical layout has been selected.

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
