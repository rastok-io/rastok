# Checked examples and historical sketches

The checkout uses **format-23 / diagnostic-recording-4**. Its
[lifecycle inventory](../architecture/lifecycle-registry-identity.md) records
the `l:loop` and `l:close` trigger movement: four gallery traces change,
while semantic hashes and package bodies remain fixed.
The preceding single Process Contract identity is tracked in the
[Connector identity inventory](../architecture/connector-rework-identity.md),
with verification in the [qualification report](../architecture/connector-rework-qualification.md).
Both Process modes use `std/process@6`. That format-22 change moved their
semantic hashes and Process Retry traces. The
[C4 qualification ledger](../architecture/retry-c4-qualification.md) is dated
evidence for the native-Retry base. External opening gates remain separate.
Author against the live
[grammar](../spec/grammar.md), [Contract sheets](../spec/stdlib.md) and
[Loop specification](../spec/loop.md), not a frozen benchmark prompt or an old
hand-derived trace. Graphs wire; Operators execute DSL handlers; Connectors
execute their Contracts. Loop contributes no port.

For a guided introduction, read the [learning path](../site/index.md): each
chapter adds a problem, explains the relevant graph, and asks you to change
one behavior and inspect the golden. The catalogue below is the reference
inventory, not the recommended reading order. The resident-process
conversation chapter includes a resident WebSocket-to-Process bridge, then
a standalone HTML/vanilla-JavaScript client served by the same App.

## Executable examples

The opening catalogue below includes the smaller reference programs as well
as the six-chapter path. The webhook page
includes the required synthetic deployment profile in its CLI command; the
HTTP model adapter also needs the profile and fixture bindings listed below.
The other examples need none. The [opening ledger](../architecture/opening-showcase.md)
tracks the six-page public gallery separately from these executable examples.
The [documentation audit](../architecture/documentation-audit-2026-09-09.md)
retains the earlier checkpoint; its bounded-join destination finding is now
corrected and regression-tested.

| Example | Demonstrates | Scenarios |
|---|---|---:|
| [gauge](../../examples/gauge.rkg) | One typed Operator and a public App output | 1 |
| [CSV line count](../../examples/csv-count.rkg) | A scripted Process `cat`, line framing, typed running count | 1 |
| [echo](../../examples/echo.rkg) | Process Call, success/timeout, explicit dispositions | 3 |
| [JSONL](../../examples/jsonl.rkg) | Structured Process frames and deterministic replay | 1 |
| [countdown](../../examples/countdown.rkg) | Operator Loop with a re-derived termination measure | 2 |
| [keyed retry](../../examples/keyed-retry.rkg) | Bob completes once, Alice retries the pinned request; keyed idle lifetime | 2 |
| [webhook to partner](../../examples/webhook-to-partner.rkg) | HttpServer ACK, validation, deduplication, native HttpClient Retry, typed per-attempt charges and privacy/secret policy | 6 |
| [WebSocket gateway](codex-websocket-gateway.rkg) | Session request/response boundary with a scripted Process job | 1 |
| [expense approval](expense-approval.rkg) | Keyed two-input join, approval/rejection and timed expiry | 3 |
| [File pipeline](../../examples/file-pipeline.rkg) | Bounded read, byte count including UTF-8, explicit empty/refusal paths, atomic replacement | 4 |
| [sensor alarm](../../examples/sensor-alarm.rkg) | Per-sensor hysteresis, a missing heartbeat, a virtual day | 3 |
| [resident dialogue](../../examples/agent-dialogue.rkg) | One JSONL Process per conversation, match mocks, exact child closure, fresh input after quit | 2 |
| [resident agent gateway](../../examples/agent-gateway.rkg) | WebSocket → per-conversation Process, app-server JSON frames, exact child shutdown, independent sockets | 2 |
| [browser chat](../../examples/agent-chat.rkg) | Embedded readable HTML, shared HTTP/WebSocket listeners, one Process per nickname, conversation-scoped Quit | 3 |
| [HTTP model adapter](../../examples/agent-http.rkg) | Projected OpenAI Responses schema, original typed usage by job/session, absent versus zero, secret boundary; no automatic inference retry | 6 |

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

The HTTP model adapter also takes `--deployment examples/agent-http.deployment.json`;
set `RASTOK_SHOWCASE_MODEL_TOKEN=lab-only-not-a-credential` and
`RASTOK_SHOWCASE_MODEL_TOKEN_VERSION=fixture-v1` for its Lab.
Simulation never contacts the configured OpenAI URL. Its local wire qualification
and persistence limits are recorded in the
[projected JSON ledger](../architecture/openai-responses-hydration.md).

The CSV example counts physical lines excluding the header; it is not a general
CSV parser for quoted multiline fields. Its [sample file](../../examples/csv-count.csv)
is illustrative; simulation uses the scripted Process lines, not filesystem I/O.

The expense example deliberately retains `decide.expired -> expired`.
`JoinExpired` is productible for a bounded join: removing its disposition now
refuses with `E-UNWIRED-OUTPUT`. Explicit `-> nothing` remains a valid,
deliberate discard.

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
