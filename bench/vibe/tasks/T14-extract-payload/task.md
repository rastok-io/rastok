# T14 — Payload extraction

Envelope records wrap the useful payload; the forwarder speaks bare
text lines.

## Interface (fixed)

- Graph: `graph extract : App`
- Connector `forwarder : Process` running `cat` with the `lines`
  codec.
- Input: `{site = <text>, payload = <text>}`.
- Output: the forwarder receives `payload` only (the codec carries
  text, not records), and its reply leaves at the boundary.

Submit one `.rkg` file containing the graph only — no `lab`.
