# T12 — Alert fan-out

A critical alert must reach both the archive and the pager — always
both, from the same decision.

## Interface (fixed)

- Graph: `graph fanout : App`
- Connectors `archive : Process` and `pager : Process`, each running
  `cat` with the `lines` codec.
- Input: `{level = <text>, message = <text>}`.
- Output: for `level == "critical"`, BOTH connectors receive the
  message text and both replies leave at the boundary. Any other
  level produces nothing anywhere.

Submit one `.rkg` file containing the graph only — no `lab`.
