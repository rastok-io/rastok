# M0 exit notes — assumed scope decisions

The deviations below are deliberate M0 scope decisions, recorded here
so they are repository facts rather than review findings. Each names
its normative home and its planned resolution.

## Deferred with a normative home

- **The sim-05 ten-record reference trace**
  ([simulation-semantics.md §13](../spec/simulation-semantics.md)) is
  not yet reproduced as a golden. The golden machinery (projection,
  seal, diff, `--bless`) is complete and pinned on the echo example;
  writing the full reference scenario is conformance work scheduled
  with the next slice.
- **`StderrLine`** ([stdlib.md §3.3](../spec/stdlib.md)): a successful
  job's stderr is currently dropped. The typed event belongs to the
  service-mode stream machinery; until then the drop is stated at the
  adapter, not silent.
- ~~`PreparedRequest` at the boundary~~ and ~~CallRef and the
  delivery reference~~ — **both closed 2026-08-06**: the boundary
  vocabulary carries the prepared request (sealed digest + exact
  bytes, one preparation, the adapter transports), and `call_ref`
  derives from `delivery_ref(edge, envelope_id)` per envelope.md §2,
  sealed autonomously in the OUTBOUND record (key 47).

## Structural debt (no semantic impact)

- `check.rs`, `runtime.rs`, `vm.rs`, and `golden.rs` have outgrown
  single-glance responsibility; the split follows the visible seams
  (checker graph/lab/envelope, runtime transaction/mailbox/vm,
  projection/codec/diff) once the semantic surface stops moving.
- The two-machine evidence runs as CI (`.github/workflows/ci.yml`,
  Linux + macOS) for every future push. The reproduction itself was
  performed on 2026-08-04 at commit `a05b77e`: a clean clone on a
  Debian 13 x86_64 host (rustc 1.97.1, throwaway toolchain, no
  system state touched) passed the full gate — fmt, clippy with
  denied warnings, 209 tests including the pinned golden traces,
  doc — ran the example against a real child process, and passed all
  three lab scenarios. The semantic hash of the example and the
  canonical trace seals were byte-identical to the macOS
  development machine's: the determinism claim held across
  operating systems.

- ~~The validated-plan newtype seam~~ — done 2026-08-07 (second
  review round): `plan::ValidatedExecutionPlan`, private field,
  `TryFrom`, no mutable access; `Vm::boot` and `plan::encode`
  accept only it, and the compiler's lowering earns it once.
