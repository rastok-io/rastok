# T02 — Critical triage

Alerts arrive as records with a severity level. Only critical alerts
may reach the on-call notifier, and the notifier receives just the
message text.

## Interface (fixed)

- Graph: `graph triage : App`
- Connector `notifier : Process` running `cat` with the `lines` codec.
- Input: `{level = <text>, message = <text>}`.
- Output: for `level == "critical"`, the notifier receives
  `message` and its reply leaves at the boundary. Any other level
  produces nothing anywhere.

Submit one `.rkg` file containing the graph only — no `lab`.
