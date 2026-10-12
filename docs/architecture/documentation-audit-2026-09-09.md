# Documentation audit — 2026-09-09

> Dated baseline, not the current delivery register. The bounded-join expiry
> defect below is resolved in the [opening ledger](opening-showcase.md).
> The v18/Process(service) follow-ups and resident gateway are tracked there
> and in [the gateway evidence](agent-gateway-showcase.md); the baseline's
> format versions, measurements and failed probes are preserved below.

Scope: whitepaper, current specifications, architecture, examples and PlantUML
sources/generated SVGs. Implementation baseline: `main` at `fd525d7`.
This pass changes documentation and example sources only: no runtime, compiler,
plan, journal, dependency, golden or semantic identity is changed.

## Current authority and coverage

The live model is Node / Graph / Executor / Connector / Extension. Every
Connector has one versioned Contract; there is no separate Family concept.
The package reader accepts **v17**, the logical type registry remains **v7**,
and the six built-in Contracts remain **@2**. Loop has three manager commands
and no contributed port; its exhaustion case uses the resolved host terminal.

| Surface | Audit result |
|---|---|
| [Whitepaper](../../WHITEPAPER.md) | Current capability table; corrected ordered-emission and Graph/Executor claims; executable references replace obsolete headline examples; future deployment, service, guest, verification and coverage sections explicitly bounded |
| [Specification index](../spec/README.md) | Live v17 pointers; package/type/Contract version registers kept distinct; current and reserved diagnostics distinguished; grammar fragments separated from complete programs |
| [Architecture](../../ARCHITECTURE.md), [components](components.md), [spec-to-code](spec-to-code.md) | Actual two-crate modules, Contract/Driver boundary, boot bindings, retained-record recovery, data policy and debugger; no imaginary public Executor/RecoveryFold trait or storage backend |
| [Examples](../examples/README.md) | Eight executable programs, including the existing CSV demo and a new complete whitepaper approval example; older sketches and seven hand-derived simulations clearly historical/future, not current goldens |
| UML views 01–04, 06, 07 | Current context, executable/browser boundary, validated compiler/runtime components, real/Lab Call sequence, module inventory and Node/Executor/Extension ownership |
| UML view 05 | Pre-build source deliberately unchanged and still labelled archival; SVG regenerated with the same renderer as the other views |
| [Release inventory](release-qualification.md) | Existing dates and external qualifications preserved; independent UI retest distinguished from automated follow-up evidence |

In particular:

- Repeated output assignments retain every executed emission, in source order;
  they are not last-write-wins. `out = .` preserves data/heritable metadata, not
  the input's system identity. `nothing` never cancels an earlier emission.
- A Graph has no application handler, mailbox or private `var`. Generic
  lifecycle records may still name its owner. Operator and Connector share
  transaction mechanics, but a Connector transition comes from its Contract.
- The current secret path is an explicit boot reference to an admitted setting,
  not a `Secret<T>` payload or arbitrary runtime environment lookup. Privacy
  controls are technical controls, not a legal-compliance certificate.
- Recovery from retained records exists through `Vm::recover_from_journal` /
  `recover_with_bindings`. `RecoveryFold` remains a design description, not a
  public type; no durable filesystem journal or cluster runtime is supplied.
- Logical deadlines are retained. Universal exact commit instants/latencies
  are not; an OTLP exporter must not fabricate them. Budget, FX, OpenCost and
  FOCUS exports remain outside the implemented accounting foundation.
- The debugger is `simulate --ui`, source-authored and initially paused. Replay
  inspects retained transitions, not external-world rewind or editable branches.
  An authored `advance` can still group VM transactions; this pass does not
  close that execution-control gap.

## Implementation finding: bounded-join expiry productibility

Follow-up: the [opening correction](opening-showcase.md) now addresses this
finding in checker and core, including `last` bounds. The observations below
describe the audit baseline, not the corrected binary; qualification of the
fix is tracked separately so the audit evidence is not rewritten.

**The destination rule is not weakened. The current implementation misses a
runtime producer in both checker and independent core productibility analysis.**

The new [expense-approval](../examples/expense-approval.rkg) program declares a
bounded two-input join and explicitly wires `decide.expired -> expired`.

```sh
cargo run -p rastok -- check docs/examples/expense-approval.rkg
cargo run -p rastok -- simulate docs/examples/expense-approval.rkg
```

Observed at the baseline:

- Check accepts, but emits `W-DEAD-OUTPUT` claiming `decide.expired` has no
  possible producer and requires no destination.
- All three scenarios pass. `approval_timeout` advances 48 hours in virtual
  time and observes exactly one `JoinExpired` on the App's `expired` output.
- A temporary copy with **only** `decide.expired -> expired` removed still
  passes check with the same warning. Its timeout scenario fails: the boundary
  observes zero matching expiry envelopes. The other two scenarios pass.

References:

- [`node_output_possible`](../../crates/rastok/src/compiler/check/routing.rs)
  considers an Operator's authored `Dispatch::may_emit` and the explicit Loop
  expiry special case, but not the timed join's runtime expiry producer.
- [`may_produce`](../../crates/rastok-core/src/plan/productibility.rs) mirrors
  that omission at the independent package gate.
- [`TransactionPreparer::join_expired`](../../crates/rastok-core/src/runtime.rs)
  actually constructs `JoinExpired`; [`Vm::evict_join_delivery`](../../crates/rastok-core/src/runtime/vm.rs)
  journals and routes that emission.

This is an explicit-destination admission defect, not just a cosmetic warning.
The checked example keeps its route. A follow-up code fix must include both
analyses and regressions for the false dead-output warning, unwired rejection,
hostile-package validation and the real timed expiry. No runtime fix or test
suppression is hidden in this documentation patch.

Reproduction identities (semantic hashes):

| Source | Hash |
|---|---|
| Full approval example | `72014bb8c723823dd5c7ae136c37ccda7f6d44330230eddc65d7879eda4c34d9` |
| Copy without the expiry route | `3eb6733d26210cb8dede43c7327337264822c847addb249ec923f98fec373df7` |

## Diagnostic status correction

[The catalogue](../spec/diagnostics.md) no longer calls the implemented R3–R9
codes future v16 output. Conversely, it marks names with no current tool
emission as reserved, including `W-DEAD-INPUT`, `E-EXTENSION-CONFLICT` and
future experiment/Saga/durability diagnostics. `E-LOOP-CONTEXT` currently names
a runtime join refusal, not a separately emitted structured compiler code.
`W-DEAD-GRAPH` does execute for inaccessible nested Graphs; unused top-level
Graph analysis remains deferred. No code is renamed or silently reassigned.

## Qualification performed

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --workspace --all-features` | **1,278 passed; 0 failed; 0 ignored; 38 suites** |
| `cargo doc --workspace --no-deps` | pass |
| Core, `--no-default-features`, `thumbv8m.main-none-eabihf` | pass |
| Core, `--no-default-features`, `thumbv7m-none-eabi` | pass |
| Core, `--no-default-features`, `riscv32imac-unknown-none-elf` | pass |
| Eight catalogue programs: `check`, then `simulate` | **13 scenarios pass**; approval warning retained and investigated above |
| Twenty benchmark reference/referee pairs | **36 scenarios pass**, no model or real external service invoked |
| `tools/diagrams/render.sh` | seven SVGs regenerated with PlantUML 1.2026.8; six current views visually reviewed |
| Local Markdown paths and heading anchors | **115 documents, 851 local links; 0 missing paths, 0 unresolved anchors**; fenced code and frozen run artifacts excluded |
| `git diff --check` | pass |

The three unchanged non-Loop witnesses also pass their existing byte-identity
tests; direct CLI hashes still match the [v17 ledger](loop-host-terminal-v17.md):
echo `5ffba3ea…`, JSONL `2f89c8b5…`, gauge `0d3ad43d…`. No golden is re-blessed.

Reference/referee replay command, from the repository root:

```sh
(
  for task in bench/vibe/tasks/T*/; do
    cargo run -q -p rastok -- simulate "$task/reference.rkg" "$task/referee.rkg" || exit 1
  done
)
```

These gates do not claim a new Cortex-M33 measurement, HTTP/WebSocket interop
rerun, Autobahn/ngtcp2 result or browser qualification. Existing evidence keeps
its original date and workload. The archived decisions, R0–R9 checkpoints,
frozen benchmark cards and old simulation derivations are not retroactively
converted into current implementation claims.
