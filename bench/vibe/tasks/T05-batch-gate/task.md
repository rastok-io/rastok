# T05 — Batch gate

Telemetry batches arrive with their readings and precomputed peak.
Empty batches are noise.

## Interface (fixed)

- Graph: `graph batches : App`
- No connector.
- Input: `{readings = <list of int>, peak = <int>}`.
- Output: for a non-empty `readings` list only:
  `{count = <int>, status = <text>}` — `count` is the number of
  readings, `status` is `"alarm"` when `peak` is strictly above 100,
  `"normal"` otherwise. An empty batch produces nothing.

Submit one `.rkg` file containing the graph only — no `lab`.
