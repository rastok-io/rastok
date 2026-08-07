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
  spike-only). It embeds the blessed **v2 plan package** of
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
| `plan::decode` (365-byte v2 package, full invariant validation + canonicity re-encode) | 4 041 | 202 k | 1.35 ms |
| relay transaction (avg of 32; min 1 775 / max 2 073) | 1 839 | **92 k** | **0.61 ms** |
| blake3 id, 64-byte frame (avg of 1000) | 90.6 | 4.5 k | 30 µs |

| Resource | Value | Criterion |
|---|---|---|
| Heap high-water mark (allocator-tracked, 32 journaled rounds) | 62 360 B (60.9 KiB) | < 128 KiB ✓ |
| Flash `.text` + `.rodata` (opt-level=z + LTO) | 163 + 30 = 193 KiB | < 1 MiB ✓ |
| Dependencies patched to build | none | none ✓ |

Notes the numbers carry:

- The relay transaction meets its criterion with a **1.6× margin**,
  not the 3× the withdrawn verdict claimed. The claim shrank; the
  fact held.
- Package decode grew from v1's 1 468 ticks to 4 041 across the
  review campaign: structural validation of every announced
  invariant (directions, canonical orders, uniqueness, the nesting
  budget, the domain-order audits) and the canonicity re-encode are
  integrity work done once at boot, and the price is stated rather
  than hidden. No criterion bounds boot cost; the transaction path
  is unchanged.
- The true high-water mark (60.9 KiB) sits above the old sampled
  figure (57.2 KiB), as a real peak must.
- The semantic hash printed by the emulated kernel is byte-identical
  to the host's: `e166cf4a…` — the identity function is the same
  function everywhere.
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
instrumentation.** All four criteria of record hold: no patched
dependencies, 92 k instructions per relay transaction against the
150 k bound (1.6×), a 60.9 KiB true high-water mark against 128 KiB,
193 KiB of flash against 1 MiB. The margins are thinner than first
claimed and are reported as measured. The option stays dormant as
planned — no embedded chantier opens. Silicon confirmation (volet B)
pending an RP2350 board; a NO-GO there would name flash wait states
as the first suspect.
