# Checked examples and historical sketches

The current package format is **v17**. Author against the live
[grammar](../spec/grammar.md), [Contract sheets](../spec/stdlib.md) and
[Loop specification](../spec/loop.md), not a frozen benchmark prompt or an old
hand-derived trace. Graphs wire; Operators execute DSL handlers; Connectors
execute their Contracts. Loop contributes no port.

## Executable examples

Checked and simulated on 2026-09-09: **8 programs, 13 passing scenarios**.
The [documentation audit](../architecture/documentation-audit-2026-09-09.md)
records commands, identities and the known timed-join diagnostic defect.

| Example | Demonstrates | Scenarios |
|---|---|---:|
| [gauge](../../examples/gauge.rkg) | One typed Operator and a public App output | 1 |
| [CSV line count](../../examples/csv-count.rkg) | A scripted Process `cat`, line framing, typed running count | 1 |
| [echo](../../examples/echo.rkg) | Process Call, success/timeout, explicit dispositions | 3 |
| [JSONL](../../examples/jsonl.rkg) | Structured Process frames and deterministic replay | 1 |
| [countdown](../../examples/countdown.rkg) | Operator Loop with a re-derived termination measure | 2 |
| [keyed retry](../../examples/keyed-retry.rkg) | Graph Loop, two keyed instances, Call timeout and retry | 1 |
| [WebSocket gateway](codex-websocket-gateway.rkg) | Session request/response boundary with a scripted Process job | 1 |
| [expense approval](expense-approval.rkg) | Keyed two-input join, approval/rejection and timed expiry | 3 |

From the repository root:

```sh
cargo run -p rastok -- check examples/csv-count.rkg
cargo run -p rastok -- simulate examples/csv-count.rkg
cargo run -p rastok --features hosted-ui -- simulate examples/csv-count.rkg --ui
```

The debugger starts paused. Edit the `.rkg` on disk, then restart the simulation;
the UI inspects execution, not source or payload authoring. A Lab mock exercises
the same Contract through controlled outcomes: passing it does **not** assert
that a real external service, credential or installed command was qualified.

The CSV example counts physical lines excluding the header; it is not a general
CSV parser for quoted multiline fields. Its [sample file](../../examples/csv-count.csv)
is illustrative; simulation uses the scripted Process lines, not filesystem I/O.

The expense example deliberately retains `decide.expired -> expired`. The
current checker incorrectly calls that timed-join output impossible, although
the timeout scenario proves `JoinExpired` is emitted. Do not remove the route
to follow the warning; see the audit's open finding.

## Historical and future designs — not passing fixtures

- [DSL design sketches](dsl-examples.md), [complex gateway](dsl-showcase-complex.rkg)
  and [Teams gateway](teams-codex-gateway.rkg) preserve pre-v16 designs. They use
  superseded syntax and/or capabilities not yet executable.
- The seven [simulation derivations](simulations/) record design reasoning,
  not current byte-exact goldens. Their per-file status states what is historical
  or still deferred. Current journal evidence lives in the checked fixtures and
  tests, particularly [instance/join scenarios](../../crates/rastok/tests/instances_lab.rs),
  [echo identity](../../crates/rastok/tests/contracts.rs),
  [JSONL identity](../../crates/rastok/tests/jsonl_fixture.rs) and
  [gauge identity](../../crates/rastok/tests/gauge_fixture.rs).
- [Benchmark cards](../../bench/vibe/prompts/) are frozen experiment inputs.
  The twenty current [reference/referee pairs](../../bench/vibe/tasks/) are
  executable, but old authored attempts and wave artifacts are not a language
  reference and are never mechanically migrated.

Historical evidence is retained as such. Updating the language does not rewrite
an old decision's date, an old benchmark score or an old causal seal.
