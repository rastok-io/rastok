# T06 — Sequence stamp

Downstream reconciliation needs every line numbered in arrival order.

## Interface (fixed)

- Graph: `graph stamp : App`
- No connector.
- Input: a text line.
- Output: `{seq = <int>, text = <text>}` — `seq` starts at 1 and
  increments by 1 per line; `text` is the incoming line.

Submit one `.rkg` file containing the graph only — no `lab`.
