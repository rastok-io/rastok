# T11 — Export under a budget

Readings are exported through an external tool that sometimes hangs.
A hung export must surface as a typed timeout, not wait forever.

## Interface (fixed)

- Graph: `graph export : App`
- Connector `exporter : Process` running `cat` with the `lines`
  codec and a declared call budget of exactly 2 seconds.
- Input: a text line, forwarded as-is to the exporter.
- Output: the exporter's reply leaves at the boundary. When the
  exporter stays silent past its budget, a `TimeoutExpired` error
  reaches the boundary error port instead — declared, not
  hand-rolled: the runtime enforces the budget you declare.

Submit one `.rkg` file containing the graph only — no `lab`.
