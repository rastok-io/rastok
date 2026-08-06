# T19 — Pressure normalization

Sensors report in bar or kilopascal; downstream stores kilopascal
only.

## Interface (fixed)

- Graph: `graph pressures : App`
- No connector.
- Input: `{value = <int>, unit = <text>}` — `unit` is `"bar"` or
  `"kpa"`.
- Output: `{kpa = <int>}` — the value multiplied by 100 when the
  unit is `"bar"`, the value unchanged when it is `"kpa"`.

Submit one `.rkg` file containing the graph only — no `lab`.
