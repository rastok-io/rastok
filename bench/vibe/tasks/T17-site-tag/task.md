# T17 — Site tagging

Bare meter readings must carry their site of origin before leaving.

## Interface (fixed)

- Graph: `graph tagging : App`
- No connector.
- Input: a text line (the raw reading).
- Output: `{site = "alpha-7", reading = <text>}` — the constant site
  tag and the incoming line.

Submit one `.rkg` file containing the graph only — no `lab`.
