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

## Method (first measurement, retained for the record)

- Firmware: `spikes/rp2350-emu` — a standalone crate outside the
  product workspace (its embedded runtime dependencies are
  spike-only). It embeds the blessed plan package of `examples/echo.rkg`
  as a `static`, opens it with `rastok_core::package::open`, boots the
  `Vm` over a `MemJournal`, and runs inject → step rounds — the
  bare-metal superloop, minus real peripherals.
- Target: `thumbv8m.main-none-eabihf` on QEMU `mps2-an505`
  (Cortex-M33), secure memory aliases, semihosting console,
  `-icount shift=0` for deterministic instruction-proportional
  virtual time.
- Tick source: SysTick, calibrated in-guest against a known
  2-instruction loop — **1 tick ≈ 20 instructions** on this setup.
  Instruction counts convert to time at 150 MHz as an OPTIMISTIC
  proxy: no flash wait states, no cache modeling. The criterion was
  hardened accordingly (< 50 k instructions per transaction, ≈ 3×
  margin under the original 1 ms).

## Numbers

| Measure | Ticks | ≈ instructions | ≈ @150 MHz |
|---|---|---|---|
| `package::open` (365-byte package) | 1 468 | 29 k | 0.20 ms |
| relay transaction (avg of 32; min 1 695 / max 1 995) | 1 761 | 35 k | **0.23 ms** |
| blake3 id, 64-byte frame (avg of 1000) | 90.6 | 1.8 k | 12 µs |

| Resource | Value | Criterion |
|---|---|---|
| Heap peak (32 journaled rounds, 256 KiB arena) | 57.2 KiB | < 128 KiB ✓ |
| Flash (`.text`, opt-level=z + LTO) | 186 KiB | < 1 MiB ✓ |
| Dependencies patched to build | none | none ✓ |

The one incident worth recording: with a 128 KiB arena, round ~60
died on a 44 KiB `Vec` doubling of the in-memory journal — the
journal grows linearly with history, as designed; a deployment
bounds it (durable journal, a later slice). The semantic hash
printed by the emulated kernel is byte-identical to the host's:
`e166cf4a…` — the identity function is the same function everywhere.

## Verdict

**WITHDRAWN (2026-08-07)** — the numbers above stand as ticks (the
raw measurements were reproduced exactly by the review), but their
instruction and time conversions used the false 20-instructions-per-
tick ratio, and the heap figure is not a high-water mark. No verdict
is in force until the corrected measurement commit replaces this
section.
