# T04 — Threshold classification

Sensor peaks must be classified before display.

## Interface (fixed)

- Graph: `graph classify : App`
- No connector.
- Input: `{peak = <int>}`.
- Output: `{status = <text>}` — `"alarm"` when `peak` is strictly
  above 100, `"normal"` otherwise.

Submit one `.rkg` file containing the graph only — no `lab`.
