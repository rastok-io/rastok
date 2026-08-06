# T16 — First three only

A commissioning window accepts the first three probe lines and
ignores the rest of the stream.

## Interface (fixed)

- Graph: `graph window : App`
- No connector.
- Input: a text line.
- Output: the line itself for the first three lines received;
  nothing for every later line.

Submit one `.rkg` file containing the graph only — no `lab`.
