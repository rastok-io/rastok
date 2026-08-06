# T09 — Alarm with hysteresis

A pressure alarm must not flap: it trips high and releases low.

## Interface (fixed)

- Graph: `graph hysteresis : App`
- No connector.
- Input: `{value = <int>}`.
- Output: `{state = <text>}` for every reading — `"tripped"` once a
  reading goes strictly above 120, holding until a reading goes
  strictly below 80, which returns the state to `"normal"`. Readings
  in between keep the current state. The initial state is
  `"normal"`.

Submit one `.rkg` file containing the graph only — no `lab`.
