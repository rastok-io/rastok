# Spike — the kernel on Cortex-M33 (emulated)

> M0.5 item 5, volet A. First measured 2026-08-06; **that
> measurement's verdict was withdrawn on 2026-08-07** — the
> adversarial review showed its calibration loop compiled to five
> instructions per iteration, not the two the conversion assumed
> (`black_box` spills through memory), so every instruction estimate
> was ≈ 2.5× low, and the reported heap peak sampled `used()` at
> chosen instants instead of tracking a true high-water mark. The
> review also noted, correctly, that the git history carried no
> numeric thresholds before the first results commit. This revision
> therefore records the criteria BELOW, in their own commit, before
> the corrected measurement lands; the next commit brings the
> instrumentation fixes and the re-measured numbers. Volet B — the
> same image on RP2350 silicon — remains pending hardware.

## Criteria of record (written before the re-measurement)

The thresholds are the original brief's, unchanged since the slice —
now stated numerically in the repository *before* the numbers:

1. **Compiles without patching any dependency.**
2. **One relay transaction < 1 ms at 150 MHz** — under the
   instruction-proportional proxy, **< 150 000 instructions**. The
   earlier self-imposed "hardened < 50 k" bound is dropped: it was
   an artifact of the false calibration, and no design decision ever
   rested on it. Margins are reported as measured, not promised.
3. **True heap high-water mark < 128 KiB** — measured by an
   allocator wrapper that records the peak at every allocation, not
   by sampling.
4. **Flash `.text` < 1 MiB.**

The calibration method of record: a counted loop written in inline
assembly — `subs` + `bne`, two instructions per iteration **by
construction**, verified against the disassembly — so the
ticks-to-instructions ratio cannot drift with compiler optimization.

## Method (corrected measurement, 2026-08-07)

- Firmware: `spikes/rp2350-emu` — a standalone crate outside the
  product workspace (its embedded runtime dependencies are
  spike-only). Its current M1.4 measurement embeds the blessed **v9
  lexical-owner package** of `examples/echo.rkg` as a `static`, opens it with
  `rastok_core::plan::decode` — which now validates
  structurally and re-encodes for canonicity — boots the `Vm` over a
  `MemJournal`, and runs inject → step rounds: the bare-metal
  superloop, minus real peripherals. Since R4 gives every selected owner a
  lazy lifecycle transaction, the harness identifies the exact `relay` / `in`
  transaction from the journal actor and trigger; lifecycle turns execute but
  are not mislabeled as relay samples.
- Target: `thumbv8m.main-none-eabihf` on QEMU `mps2-an505`
  (Cortex-M33), secure memory aliases, semihosting console,
  `-icount shift=0` for deterministic instruction-proportional
  virtual time.
- Calibration: the inline-assembly loop (2 instructions per
  iteration by construction, confirmed against the disassembly —
  `subs r3, r3, #1 ; bne`) measures **1 tick ≈ 50 instructions** on
  this setup (39 999 ticks for 2 000 000 instructions). The first
  measurement assumed 20 — its Rust loop compiled to 5 instructions
  per iteration; every prior estimate was ≈ 2.5× low.
- Heap: the global allocator is wrapped, and the peak is recorded
  inside every successful allocation — a true high-water mark, not
  a sampled `used()`.
- Instruction counts convert to time at 150 MHz as an OPTIMISTIC
  proxy: no flash wait states, no cache modeling. Volet B on silicon
  is what verifies wall-clock.

### Compile-only portability companions

The Cortex-M33 image remains the one executable embedded proof. CI also
compiles `rastok-core` with default features disabled for
`thumbv7m-none-eabi` (Cortex-M3: older Armv7-M baseline without an FPU) and
`riscv32imac-unknown-none-elf` (generic non-Arm 32-bit baseline). These two
checks prove that the semantic kernel has no accidental Armv8-M, hardware
floating-point or Arm-only dependency; they make no execution, timing,
peripheral or silicon claim.

## Numbers (corrected)

| Measure | Ticks | ≈ instructions | ≈ @150 MHz |
|---|---|---|---|
| `plan::decode` (927-byte v9 lexical-owner package; re-measured after M1.4 R4 on 2026-08-21) | 15 345 | 767 k | 5.12 ms |
| — re-measured 2026-08-09, rounds four and five: +109 decode ticks (the pinned-edge audit at validation) and ~14 per relay transaction (the crossing-pin judge at construction + the pre-commit choke point scanning for watching pins; the FINAL gate converts the value only when a pin watches its fan-out — construction-level checks and expression bindings still materialize handler values as they always did) — priced, no criterion near | | | |
| relay transaction (avg of 32 exact actor/trigger samples; re-measured 2026-08-21 after R4) | 2 767 (min 2 660 / max 2 979) | **138 k avg / 149 k max** | **0.92 ms avg / 0.99 ms max** |
| blake3 id, 64-byte frame (avg of 1000) | 90.8 | 4.5 k | 30 µs |

| Resource | Value | Criterion |
|---|---|---|
| Heap high-water mark (allocator-tracked, 32 journaled relay rounds plus their owner lifecycle facts; re-measured 2026-08-21 after R4) | 104 124 B (101.7 KiB) | < 128 KiB ✓ |
| Flash `.text` + `.rodata` (opt-level=z + LTO) | 417 492 + 45 024 = 462 516 B (451.7 KiB; re-measured 2026-08-21 after R4) | < 1 MiB ✓ |
| Dependencies patched to build | none | none ✓ |

Notes the numbers carry:

- The slowest measured relay transaction meets its criterion with a **1.007× margin**,
  not the 3× the withdrawn verdict claimed. The claim shrank; the
  fact held.
- Package decode grew from v1's 1 468 ticks to 15 334 across the
  review campaign: structural validation of every announced
  invariant (directions, canonical orders, global uniqueness, the
  nesting budget, the domain-order audits) and the canonicity
  re-encode are
  integrity work done once at boot, and the price is stated rather
  than hidden. The 2026-08-14 logical type landing leaves the v6
  package and semantic hash byte-identical. B2 then adds 1 185 decode
  ticks to derive and validate the complete effective port table, 54 ticks
  on the measured relay path, and 464 bytes at the heap peak; the validator
  does not substitute `variant` merely to save these resources. No
  criterion bounds boot cost; the transaction path remains below its
  recorded bound.
- The B0–B3 adversarial closure temporarily recorded the VM's implicit
  `Operator.err` producer in the resolved table. Final M1.3 review removed
  the premise: pure Operator engine faults never become graph values, and an
  explicit `err` is only a domain output. The package is re-measured in the
  table above without that invented row; its semantic hash and causal behavior
  remain unchanged.
- B3 leaves the v6 package, heap peak and semantic hash unchanged. Linking
  the expression narrowing and union-consumption paths adds 55 ticks to the
  measured relay and 15 508 bytes to `.text + .rodata`; both costs are stated
  before v7 activates those trees in packages.
- The B1 completion pass still leaves the v6 package and semantic hash
  unchanged. The complete materialized value algebra adds 47 decode ticks,
  53 relay ticks, 2 688 bytes at the heap peak and 15 704 bytes of
  `.text + .rodata`. Those are implementation costs of admitting the exact
  JSON, Avro and Arrow value families in the kernel; v6 still refuses their
  unpublished wire forms.
- B5 replaces the travelling Process `error_set` with four complete port
  rows and proves their type/direction/routing contract without a compiler
  Registry. The package shrinks from 430 B to 416 B, while the stronger boot
  proof costs 1 110 decode ticks; the measured relay changes by 10 ticks,
  heap peak by 276 B, and linked flash shrinks by 508 B. The transport and
  identity moves are declared; the causal trace remains unchanged.
- B7 keeps the package at 416 B, replaces the success/failure reply with one
  ordered completion, and journals unit-framed `out*`, independent `err*`,
  then exact `exit`. Relative to B5, decode changes by −2 ticks, relay by −20
  ticks, and the heap peak by −512 B; the linked completion/framing paths add
  2 988 B of `.text + .rodata`. The Process hash and causal trace move for
  those declared semantic causes.
- B8 publishes the complete v7 type/value/expression registry and carries the
  mandatory total resolved-port table. The echo package grows from 416 B to
  882 B; relative to B7, decode costs 7 402 additional ticks, the relay path
  changes by 7 ticks, the heap peak grows by 1 556 B, and linked flash grows
  by 25 496 B. The one-time boot cost is not a relay criterion; all recorded
  resource bounds continue to hold.
- Final M1.3 review removes the lowering-only Operator `err` row and its
  synthetic fault path. Relative to B9, the echo package shrinks 882 B →
  861 B, decode drops 610 ticks, relay drops 5 ticks, heap peak drops 80 B,
  and linked flash drops 1 500 B. The semantic hash and causal trace remain
  byte-identical.
- The former M1.4 P1–P3 draft moved only the package container to unpublished
  v8 for this unkeyed graph: its 861-byte size and semantic hash remained
  unchanged, while the checksum reflected the version. D-2026-08-18-1
  superseded that named-partition transport. R2 now publishes v9 with a
  canonical lexical-owner table and an owner scope on every edge. The package
  grows 861 B → 927 B, decode costs 1 191 additional ticks, the measured relay
  changes by −1 tick, heap peak grows by 448 B, and linked flash grows by
  23 192 B. The semantic hash and causal trace remain byte-identical: this is
  the declared package-only identity move. The R3 row below separately prices
  the first executable hierarchical routing and lifecycle-reference machinery.
- The R2 adversarial-validation closure leaves the 927-byte package, semantic
  hash, and causal trace byte-identical. Re-deriving a key type from the full
  immutable expression signature adds 4 decode ticks and links 11 704 B of
  checker code; relay's +3-tick average/+1-tick max is code-layout jitter,
  while removing the false RAM generation lowers the measured heap peak by
  512 B.
- R3 leaves the 927-byte package and semantic hash fixed, but deliberately
  moves causal traces: every route, firing and consumed cursor now carries a
  complete hierarchical instance reference. Relative to reviewed R2, decode
  adds 11 ticks, relay adds 313 ticks on average, heap peak adds 8 556 B, and
  linked flash adds 25 660 B. The review closure changes the address spelling
  from dotted owner text to structural segment arrays. The first direct
  implementation exceeded the relay criterion (3 764 ticks): the measured fix
  keeps the generation index
  as a replay-derived journal projection, shares immutable canonical address
  bytes across references, transports only lightweight references for live
  bundles, and indexes static owner chains once. No semantic shortcut was
  taken; the final maximum is 2 945 ticks.
- R4 leaves the 927-byte package and semantic hash fixed and deliberately
  moves the four causal traces: every selected owner now records lazy
  `l:start`, and the five lifetime profiles execute with generation-fenced
  deadlines and inner-first closure. Relative to reviewed R3, the exact relay
  sample adds 30 ticks on average and 34 ticks at maximum. The first direct
  build exceeded the relay bound because non-tumbling singleton routes rebuilt
  their canonical address and journal generation on every delivery; the
  measured correction reuses the already-validated live reference for that
  semantically fixed address. The heap peak adds 25 000 B: the three universal
  owner lifecycle facts push the in-memory journal across its next `Vec`
  capacity step. Linked lifetime machinery adds 15 580 B. No package byte or
  semantic hash moves.
- Flash grew from 193 KiB to 451.7 KiB (`.text` 417 492 B +
  `.rodata` 45 024 B, re-measured 2026-08-21): the consolidated
  reform's decoders, the jsonl codec's canonical writer and now the
  complete logical-type algebra, inference solver and validator link into
  the kernel even for a `lines` plan — the core is one crate, and dead-code
  elimination keeps what the plan decoder can reach. The 1 MiB criterion holds with a
  2.49× margin; trimming by feature gate is noted, not needed.
- The true high-water mark (101.7 KiB) sits above the old sampled
  figure (57.2 KiB), as a real peak must; the first M1 lift added
  the ingress port to every inbound record (+688 B over 32 rounds),
  and the consolidated reform's five-field spine and ext zone
  roughly cancelled out.
- The semantic hash printed by the emulated kernel is byte-identical
  to the host's: `514d4ab7…` — the identity function is the same
  function everywhere. (Six declared identity moves so far: with
  D-2026-08-10-7 — a compiled tree seals its static type with its
  root — then twice on 2026-08-12 with D-2026-08-12-1: the default
  injection made the EFFECTIVE sheet identity, and the Process
  error set gained `CaptureFailed`; then B5 retired that temporary set in
  favor of complete resolved port rows and materialized `err_codec`; B7 made
  Process framing and atomic typed completion part of executable identity;
  B8 sealed complete resolved types and removed the last transport bridges.)
- The journal-growth incident of the first run stands: with a
  128 KiB arena, a ~44 KiB `Vec` doubling of the in-memory journal
  dies around round 60 — linear history growth, as designed; a
  deployment bounds it (durable journal, a later slice).

## Reproduction

From the repository root (the crate must be built from its own
directory — its `.cargo/config.toml` sets the target and linker
script, and Cargo only reads that file from the working directory):

```sh
cd spikes/rp2350-emu
cargo build --release
qemu-system-arm -M mps2-an505 -cpu cortex-m33 \
  -kernel target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  -nographic -monitor none -serial none \
  -semihosting-config enable=on,target=native -icount shift=0
```

Calibration-loop verification (expect `subs` + `bne`, two
instructions, branching to itself):

```sh
llvm-objdump -d target/thumbv8m.main-none-eabihf/release/rastok-spike-rp2350 \
  | grep -A1 'subs.*#0x1$' | head -4
```

## Verdict

**GO (provisional), re-issued 2026-08-07 on the corrected
instrumentation and last re-measured 2026-08-21 after M1.4 R4.**
These measurements price complete hierarchical routing, causal instance
references, and the executable lifetime machine; bounded Closing remains a
separate R5 measurement. All four criteria of record hold: no patched
dependencies, 149 k instructions for the slowest measured relay transaction
against the 150 k bound (1.007×), a 101.7 KiB true high-water mark against
128 KiB, and 451.7 KiB of flash against 1 MiB. The
margins are thinner than first
claimed and are reported as measured. The option stays dormant as
planned — no embedded chantier opens. Silicon confirmation (volet B)
pending an RP2350 board; a NO-GO there would name flash wait states
as the first suspect.
