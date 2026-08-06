# T07 — Running total

Meter pulses arrive as deltas; billing wants the running total.

## Interface (fixed)

- Graph: `graph totals : App`
- No connector.
- Input: `{amount = <int>}`.
- Output: `{total = <int>}` — the sum of every `amount` received so
  far, including this one.

Submit one `.rkg` file containing the graph only — no `lab`.
