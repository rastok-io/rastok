# T10 — Failure streak

Health probes report success or failure; operations watches the
consecutive-failure count.

## Interface (fixed)

- Graph: `graph streaks : App`
- No connector.
- Input: `{ok = <bool>}`.
- Output: `{streak = <int>}` for every probe — the number of
  consecutive failures ending at this probe; a success resets it
  to 0.

Submit one `.rkg` file containing the graph only — no `lab`.
