# T20 — Guarded export

Only readings marked valid may be exported, the export runs under a
1-second budget, and a hung export must surface as a typed timeout.
This is T02's gate and T11's budget in one graph — the shape of a
real edge exporter.

## Interface (fixed)

- Graph: `graph guarded : App`
- Connector `uplink : Process` running `cat` with the `lines` codec
  and a declared call budget of exactly 1 second.
- Input: `{valid = <bool>, reading = <text>}`.
- Output: for `valid == true`, the uplink receives `reading` and its
  reply leaves at the boundary; when the uplink hangs past its
  budget, a `TimeoutExpired` error reaches the boundary error port.
  For `valid == false`, nothing happens anywhere.

Submit one `.rkg` file containing the graph only — no `lab`.
