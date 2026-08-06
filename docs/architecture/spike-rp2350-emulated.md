# Spike verdict — the kernel on Cortex-M33 (emulated), 2026-08-06

> M0.5 item 5, volet A. Criteria were written before measurement
> (m0.5-slice.md); this note records method, numbers, and verdict.
> Volet B — the same image on RP2350 silicon — is pending hardware
> and will append two lines to this note, not rewrite it.

## Method

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

**GO (provisional).** All four criteria hold with margin; the
transaction proxy sits 30 % under its hardened bound, which itself
is 3× under the original wall-clock criterion. The option stays
dormant as planned — no embedded chantier opens. Silicon
confirmation (volet B) pending an RP2350 board; a NO-GO there would
name flash wait states as the first suspect.
