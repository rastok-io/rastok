# T15 — Bounds check

Each reading carries its own acceptance band; flag whether it sits
inside.

## Interface (fixed)

- Graph: `graph bounds : App`
- No connector.
- Input: `{value = <int>, low = <int>, high = <int>}`.
- Output: `{ok = <bool>}` — true exactly when
  `low <= value <= high` (both ends inclusive).

Submit one `.rkg` file containing the graph only — no `lab`.
