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
  spike-only). It embeds the blessed **v6 plan package** of
  `examples/echo.rkg` as a `static`, opens it with
  `rastok_core::plan::decode` — which now validates
  structurally and re-encodes for canonicity — boots the `Vm` over a
  `MemJournal`, and runs inject → step rounds: the bare-metal
  superloop, minus real peripherals.
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

## Numbers (corrected)

| Measure | Ticks | ≈ instructions | ≈ @150 MHz |
|---|---|---|---|
| `plan::decode` (430-byte v6 package, full invariant validation + effective-port resolution + canonicity re-encode; re-measured 2026-08-14 after the M1.3 B1 completion pass) | 6 086 | 304 k | 2.03 ms |
| — re-measured 2026-08-09, rounds four and five: +109 decode ticks (the pinned-edge audit at validation) and ~14 per relay transaction (the crossing-pin judge at construction + the pre-commit choke point scanning for watching pins; the FINAL gate converts the value only when a pin watches its fan-out — construction-level checks and expression bindings still materialize handler values as they always did) — priced, no criterion near | | | |
| relay transaction (avg of 32; re-measured 2026-08-14 — deterministic under this QEMU icount run) | 2 323 (min 2 270 / max 2 569) | **116 k** | **0.77 ms** |
| blake3 id, 64-byte frame (avg of 1000) | 90.8 | 4.5 k | 30 µs |

| Resource | Value | Criterion |
|---|---|---|
| Heap high-water mark (allocator-tracked, 32 journaled rounds; re-measured 2026-08-14 after the B1 completion pass) | 66 520 B (65.0 KiB) | < 128 KiB ✓ |
| Flash `.text` + `.rodata` (opt-level=z + LTO) | 297 268 + 38 632 = 335 900 B (328.0 KiB; re-measured 2026-08-14 after the B1 completion pass) | < 1 MiB ✓ |
| Dependencies patched to build | none | none ✓ |

Notes the numbers carry:

- The relay transaction meets its criterion with a **1.29× margin**,
  not the 3× the withdrawn verdict claimed. The claim shrank; the
  fact held.
- Package decode grew from v1's 1 468 ticks to 6 086 across the
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
- Flash grew from 193 KiB to 328.0 KiB (`.text` 297 268 B +
  `.rodata` 38 632 B, re-measured 2026-08-14): the consolidated
  reform's decoders, the jsonl codec's canonical writer and now the
  complete logical-type algebra, inference solver and validator link into
  the kernel even for a `lines` plan — the core is one crate, and dead-code
  elimination keeps what the plan decoder can reach. The 1 MiB criterion holds with a
  3.12× margin; trimming by feature gate is noted, not needed.
- The true high-water mark (65.0 KiB) sits above the old sampled
  figure (57.2 KiB), as a real peak must; the first M1 lift added
  the ingress port to every inbound record (+688 B over 32 rounds),
  and the consolidated reform's five-field spine and ext zone
  roughly cancelled out.
- The semantic hash printed by the emulated kernel is byte-identical
  to the host's: `c75b840d…` — the identity function is the same
  function everywhere. (Three declared identity moves so far: with
  D-2026-08-10-7 — a compiled tree seals its static type with its
  root — then twice on 2026-08-12 with D-2026-08-12-1: the default
  injection made the EFFECTIVE sheet identity, and the Process
  error set gained `CaptureFailed`, which travels in the plan.)
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
instrumentation and re-measured 2026-08-14 on package v6.** All four
criteria of record hold: no patched dependencies, 116 k instructions
per relay transaction against the 150 k bound (1.29×), a 65.0 KiB true
high-water mark against 128 KiB, 328.0 KiB of flash against 1 MiB. The
margins are thinner than first
claimed and are reported as measured. The option stays dormant as
planned — no embedded chantier opens. Silicon confirmation (volet B)
pending an RP2350 board; a NO-GO there would name flash wait states
as the first suspect.
