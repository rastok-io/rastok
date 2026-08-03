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
- **`PreparedRequest` at the boundary**: the machine and the adapter
  share one pure preparation function (`connectors::prepare`), which
  guarantees sealed-equals-sent by construction, but the VM thereby
  depends on the adapter module. Carrying prepared bytes in the
  boundary vocabulary removes the dependency and the double
  computation; planned with the service-mode rework.
- **Budget arming on a double edge**: two edges from one emission to
  the SAME connector collapse to one sealed budget obligation
  (`timers` dedup) while the scheduler arms per delivery. Reachable
  only in a degenerate wiring; resolved when multi-port connector
  requests are specified.

## Structural debt (no semantic impact)

- `check.rs`, `runtime.rs`, `vm.rs`, and `golden.rs` have outgrown
  single-glance responsibility; the split follows the visible seams
  (checker graph/lab/envelope, runtime transaction/mailbox/vm,
  projection/codec/diff) once the semantic surface stops moving.
- The two-machine evidence runs as CI (`.github/workflows/ci.yml`,
  Linux + macOS); the claim is made by a published green run, not by
  the file's presence.
