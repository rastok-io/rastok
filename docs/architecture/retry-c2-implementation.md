# C2 — source and validated-plan implementation

15 September 2026. **In progress; Retry is not executable.** The
[C1 inventory](retry-c1-identity.md) owns the proposed v19/v3 cutover.
Preparatory changes below preserve executable v18 and recording v2; no
Contract major, journal tag or golden has moved.

## 1. Exact Session opening authority

Session direction is now a core Contract fact, used by both live execution
and prefix recovery. It is not inferred from `effect`, attribute names or
duplex roles. The internal closed query returns Peer for WebSocketServer,
Local for WebSocketClient and Process service, and no authority for an unknown
Contract or inconsistent roles/exchange. No transported field or public trait
is added.

A peer cannot open a client or resident process. An uncorrelated server write
is consumed without opening a client or guessing the sole live peer. Recovery
uses the same rule for openings and writes and does not re-offer old I/O.
The existing `effect` attribute/transport remains until the single cutover;
removing its Session-direction use is a prerequisite, not that cutover.

Evidence at this checkpoint:

- the core identity/roles/exchange matrix passes (four selected tests);
- twelve `machine_vm` Session tests pass, including new peer-root refusal
  before journaling and orphan-write parity with/without recovery;
- the five focused Contract, gauge, JSONL, resident Process and exchange
  recovery suites pass (101 tests, no blessing enabled);
- tests compare retained records, not only an in-memory resource count.

These tests do not qualify Retry scheduling/accounting.

## 2. Contract-owned frame case sets

`FrameProduction::ExactlyOneOn` now carries a nonempty canonical set of
terminal names instead of one name. The VM checks each frame count against
the selected terminal; Call Loop and Graph Loop proofs use the same set when
combining frame and terminal outcomes. A frame is still externally visible
for the repetition check. The standard Contract tables remain the authority,
not a new serialized production claim.

The ten core Call-profile tests pass. Added tests exercise two response cases,
zero/one/extra frames, stream/never profiles and canonical/declared case sets.
Existing HttpClient and File read still select only `Completed`; Process job
retains its independently bounded streams. In particular, HTTP 500 is **still
Completed in the current Contract**. ServerError, its typed status checks and
two-case cardinality land with the declared Contract-major cutover, not here.

## 3. Preparatory qualification

All required workspace checks pass on this batch:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
env -u RASTOK_BLESS cargo test --workspace --all-features --quiet
cargo doc --workspace --no-deps
```

The full run reports **1,398 passed, zero failed, zero ignored** across 47
test groups (including the empty binary/doc groups). Existing semantic hashes,
package fixtures and causal seals pass without re-blessing. Core also checks
on `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf` and
`riscv32imac-unknown-none-elf`. Local documentation links/fences and
`git diff --check` pass. Browser/JavaScript and independent protocol campaigns
were not rerun; no UI or physical Driver code changed.

The Cortex-M33 spike was rebuilt from `spikes/rp2350-emu` with
`cargo build --release --locked`, then run using the
[QEMU recipe](spike-rp2350-emulated.md#reproduction). `llvm-size -A` measured
the resulting release ELF:

| Measurement | Result | Compared with pre-C2 baseline |
|---|---|---|
| peak heap | 111,732 B; 19,340 B below 128 KiB | unchanged |
| `.text` + `.rodata` | 882,788 + 69,872 = 952,660 B | +632 B; 95,916 B below 1 MiB |
| Record / Inbound / Transaction / Outbound layouts | 8 / 216 / 224 / 8 B | unchanged |
| relay ticks, average / minimum / maximum | 1,907 / 1,858 / 2,092 | unchanged |
| retained records | 67, `SPIKE_OK` | echo semantic hash unchanged |

This is the existing non-Retry echo workload. It proves neither held-attempt
memory nor new closure-field overhead; those measurements remain cutover/C3
gates. No gate, package fixture or golden was changed.

## 4. Remaining C2 gates

- HTTP ServerError and its declared Contract/type cutover, including decoded
  status boundaries and frame cardinality in Driver, Lab and core validation.
- Source Retry settings and plan-owned idempotence; `W-LOOP-REPEAT` and its
  positive/negative/sibling/canonical-order tests.
- Remove global effect and Intent.effect without duplicating the guarantee.
- Carry all C1 packet, reference, closure-time and recording changes in the
  single cutover, with independent hostile validation.
- Keep named execution refusals for unfinished C3 paths; no check-green
  construct that every runtime must reject.

Runtime attempt publication, shared recurrence/recovery and accounting are
C3. Example/golden/UI migration and final qualification remain C4.
