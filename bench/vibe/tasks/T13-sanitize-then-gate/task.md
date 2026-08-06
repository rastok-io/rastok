# T13 — Sanitize, then gate

Raw sensor values must be clamped into range before the threshold
decision — two separate responsibilities, two operators.

## Interface (fixed)

- Graph: `graph pipeline : App`
- No connector. Two operators in a chain: the first clamps, the
  second gates.
- Input: `{value = <int>}`.
- Output: the clamped value record `{value = <int>}`, only when the
  clamped value is strictly above 150. Clamping bounds `value` into
  [0, 200]. Anything at or below 150 after clamping produces
  nothing.

Submit one `.rkg` file containing the graph only — no `lab`.
