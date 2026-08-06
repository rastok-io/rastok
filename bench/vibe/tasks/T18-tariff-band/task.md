# T18 — Tariff banding

Spot prices are banded for display: low, mid, high.

## Interface (fixed)

- Graph: `graph tariffs : App`
- No connector.
- Input: `{price = <int>}`.
- Output: `{band = <text>}` — `"low"` for a price strictly below
  50, `"high"` for a price strictly above 150, `"mid"` otherwise.

Submit one `.rkg` file containing the graph only — no `lab`.
