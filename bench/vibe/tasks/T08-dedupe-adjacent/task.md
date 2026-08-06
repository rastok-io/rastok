# T08 — Adjacent deduplication

A chatty device repeats its status line; downstream wants changes
only.

## Interface (fixed)

- Graph: `graph dedupe : App`
- No connector.
- Input: a text line.
- Output: the line itself, but only when it differs from the
  previously received line. The first line always passes. A repeat
  produces nothing.

Submit one `.rkg` file containing the graph only — no `lab`.
