# T01 — Log relay

Field devices emit free-text status lines. Forward every non-empty
line to the site logger; drop empty lines silently.

## Interface (fixed — the referee depends on these names)

- Graph: `graph relay : App`
- Connector `logger : Process` running `cat` with the `lines` codec.
- Input: a text line at the boundary.
- Output: the logger's reply leaves at the boundary; empty input
  produces nothing anywhere (the logger must not even be called).

Submit one `.rkg` file containing the graph only — no `lab`.
