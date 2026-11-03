# M1 implementation status, in detail

> Moved from the README on 25 October 2026, unchanged except for link paths.
> The README keeps the summary; this page keeps the paragraph-level record of
> what each milestone made executable, the command-line options and the
> stated limits. Format and identity movements are registered in the
> [current plan package](../spec/identity.md); the external evidence still
> open is in the [opening ledger](opening-showcase.md) and the
> [release-qualification inventory](release-qualification.md).

## Package format and ledgers

The [current package format](../spec/identity.md), with one Node table and
Graph-owned wiring ([representation inventory](node-realignment-identity.md)), carries recursive Dispatch, case routing,
ordered multi-emission and command-based Operator, Graph and outbound Call
Loops. Independent core validation re-derives their contracts, routes and
proofs. Recovery reconstructs recorded deliveries, state, commands, exchanges
and original deadlines without re-executing producers or repeating I/O.
Nested runs, same-context joins, annotations and entry-preserving measures
are qualified within the [named Loop proof subset](../spec/loop.md#16-deferred-by-name).
The [R9 qualification ledger](rework-r9-qualification.md)
records the v16 rework; the subsequent
[v17 ledger](loop-host-terminal-v17.md) records the
host-terminal amendment and its unchanged non-Loop witnesses. The
[v18 closure ledger](instance-stop-command.md) adds
`instance.stop` and `child.instance.stop`: exact-generation lifecycle requests,
not ports or spatial edges. The subsequent
[resident Process contract](process-service.md) uses those
commands and the existing Session exchange: one child per instance, streamed
lines/JSONL, and no automatic restart or protocol-specific RPC.

## Status and known limits

Rastok is at an early experimental stage. Its specification and
implementation architecture are under active review, and the runtime
does not yet make a production-readiness claim.

Opening is planned for **5 November 2026**, on explicit release instruction.
The independent HTTP/3 server test against ngtcp2, official Autobahn suite,
and measurement of the work needed to add a Connector remain **open and
non-blocking for that opening**. The debugger's final browser qualification
is still blocking. None is implied by a green Rust test suite. M1 has no
durable deployment journal, Kafka or SQL Connector; diagnostic recordings
are not durable recovery or share-safe exports. See the
[opening ledger](opening-showcase.md).

Format 24 separates logical time from civil dates. Logical
instants and durations use signed 64-bit nanoseconds; `timestamp` and
`timestamp_ntz` use signed seconds plus a nanosecond fraction, including dates
before 1678 and through year 9999. The journal retains the civil origin used
by `now()` and epoch-aligned tumbling boxes. The approved witness movement
and qualification are recorded in the
[time-carrier inventory](time-carriers-identity.md).

The current executable proof is one continuous path:

```text
Rastok DSL source
  -> validated executable plan
  -> deterministic state machine
  -> Execution driven by one hosted Worker and an in-memory journal
  -> mixed real Process and File boundaries selected through one Driver registry
  -> journaled, auditable multi-Contract Lab scenarios with useful golden diffs
  -> optional paused simulation debugger with recorded-transition inspection
```

Connector Contracts have stable semantic ids and inbound/outbound directions.
Current binaries compose Drivers explicitly at build/assembly time. A deployment
selects an implementation by Connector Node path; when exactly one compatible
implementation is registered, it is selected without an explicit name. Missing,
incompatible or ambiguous implementations refuse before any effect. Minimal,
embedded and full builds may link different sets without changing plan identity.
Run `rastok connectors` (or `rastok connectors --json`) to inspect the standard
Contracts, modes, ports, attributes, installed implementations, Lab support and
named conformance gaps without starting a Driver. Contributors follow the
[File-based Connector Driver guide](connector-drivers.md).
Rastok does not yet ship a plugin loader or freeze a native ABI.

M1.6 is closed. Four exact HTTP/WebSocket sheets cross source, Registry-free
plan validation, Lab and live Drivers. HttpClient and HttpServer execute the
same Call contracts over HTTP/1.1, HTTP/2 and HTTP/3. WebSocketClient and
WebSocketServer execute one generic Session over HTTP/1.1 Upgrade, HTTP/2
Extended CONNECT and HTTP/3 Extended CONNECT: complete messages travel in
either direction and one typed `exit` closes the connection. All hosted I/O
runs behind one bounded private pump using Hyper/Rustls, quiche/BoringSSL and
Tungstenite; none of those dependencies enters `rastok-core` or a no-std
build. Static feature presets compose reduced or full Driver registries
without changing plan semantics. The executable
[WebSocket-to-job example](../examples/codex-websocket-gateway.rkg) and the
[M1.6 exit report](m1.6-exit-report.md) record the complete
proof. The single
[release-qualification inventory](release-qualification.md)
records executed independent HTTP/WebSocket interop and the remaining
ngtcp2/Autobahn qualifications without conflating them with workspace tests.

The later [resident agent gateway](../../examples/agent-gateway.rkg) keeps one JSONL
child per conversation. Its [gallery chapter](../site/examples/06-agent-gateway.md)
and [evidence](agent-gateway-showcase.md) distinguish scripted
protocol replies and real local composition tests from public authentication,
live model qualification or per-job usage accounting.

M1.7 is closed as historical v14/v15 evidence: it proved fuel-bounded and
directly ranked temporal recurrence, logical delay, typed exhaustion,
concurrent runs and stale-wake fencing without a spatial back-edge. The
[M1.7 exit report](m1.7-exit-report.md) remains the exact
record of that implementation. D-2026-09-05-1 now supersedes its Graph-handler
and port-shaped control model: v16 makes Graph passive, shares transaction
mechanics between Operator and Connector, identifies a run from its exact
opening admission, and represents Next/Again/Done as durable manager commands.
The frozen [R0 algebra](rework-r0-algebra.md),
[reference traces](rework-r0-reference-traces.md), and
[R1 host contracts](rework-r1-host-contracts.md) lead to the
[R2 review handoff](rework-r2-supersession.md). The [R9 qualification](rework-r9-qualification.md)
links that design to the executable host and recovery tests.

M1.8 closed on categorical package v15; its policy guarantees remain in the
current implementation. The source front end
recognizes `@sensitive`, `@secret`, `@personal` and `@measure`,
resolves aliases into canonical schema paths and transports the sparse policy
tables through checked HIR, package decode and Registry-free validation.
Quantities use exactly `@measure(id = amount, unit = "USD")`: money is an
ordinary unit-bearing value, not a separate interpretation. The former
`@cost` spelling and currency/basis arguments refuse with migration diagnostics.
Invalid or forged annotations refuse by exact path. A3 independently derives
effective policy across copies, projections, calculations, guards, state and
Loop recurrence; accounting meanings never merge ambiguously, and terminal
presentation redacts before formatting. `@personal` remains orthogonal to
confidentiality, preserves the GDPR Articles 9/10 regimes and an exact
data-subject identifier role, and redacts by default. A4 now admits personal
data only with a complete attested run-scoped privacy profile. Exact
`env()`/`secret()` references bind HttpClient URL and bearer settings through
the same real/Lab descriptor; physical secret bytes enter neither source,
profile JSON, CLI, `Value`, package nor journal. Raw incoming `@secret` fields
remain fail-closed. The annotation migration does not change unannotated semantic hashes or
original causal DATA. The separate Retry/Contract cutover and its exact
witness movements are inventoried in the
[C4 qualification ledger](retry-c4-qualification.md). Every causal record
also yields a typed recorded observation with exact flow/instance/Call/Session
correlation. `rastok run --json-logs` projects that stream through a bounded
never-block queue; payloads are omitted by default and classified values are
redacted before JSON formatting. Export loss changes neither execution nor
the recorded facts. Measurements stay in their original typed envelopes;
the associated validated plan retains their annotations. Signed quantities
and reported amounts remain exact, with their units and original causal
provenance; a currency code is simply a unit. Under D-2026-09-16-7, M1 implements no report, automatic
pricing or subset arithmetic. Existing profile attestations keep their
representation, but no tariff is automatically applied. The ordered
work is tracked in the
[M1.8 backlog](m1.8-observability-backlog.md), and the
[exit report](m1.8-exit-report.md) owns the identity,
portability and remaining-limit ledger.
The optional OpenTelemetry transport is explicitly deferred until exact
temporal observations exist; no exporter timestamp is presented as commit
time.

The current implementation admits finite outbound Call `+ Retry`
through the public compiler, Lab, runner and diagnostic replay. It repeats the
same prepared operation, retains each attempt and publishes only the final
result. Instances distinguishes operations/attempts from Loop applications;
held envelopes remain inspectable with normal redaction. Unsupported Retry
hosts still refuse. The common fixture migration and qualification are recorded
in the [C4 ledger](retry-c4-qualification.md); opening gates
remain separate.

The same implementation admits descriptive annotations
declared with `type note = @struct { text: text }` and applied as
`text @note(text = "Caller input")`. Definition identity and typed constant
arguments are retained and independently checked; a lookalike of a standard
annotation gains no security or accounting authority. Ordinary flat data and
unused declarations stay identity-neutral. Additive business quantities now
use simple ids and explicit units, for example
`int @measure(id = carrots, unit = "item")`. Signed integers and exact decimals
retain individual positive, zero and negative samples; decimal quantities are not implicitly
money. Amounts use that same two-field schema, for example
`decimal(18,6) @measure(id = refund, unit = "EUR")`; their ids remain distinct
in the recorded metadata. Generic subsets, reports, pricing and the future
Budget Extension are outside M1. The implicit-unit registry is retired: even a
dotted id needs an explicit unit. Standard definitions and aliases use the
ordinary type system. The [common qualification](retry-c4-qualification.md)
records that migration separately from external M1 opening gates.

The visual debugger is available through `simulate --ui`. It starts paused,
uses the checked scenario on disk, and retains a read-only transition timeline.
Back/forward inspection does not fork or mutate the simulation;
Reset starts a fresh session and preserves the old history. The embedded page
shows real compound topology, current mailbox FIFOs (`≡ n`), instances
(`⧉ n`), source definitions, Loop-run state dependencies, redacted `META`/
`DATA`. Manager commands are described as commands,
never invented ports or spatial edges. Replay presents recorded transitions at
one or four events per second. Decisions, causal links, pending obligations
and assertion evidence explain the selected journal prefix. The canvas
pans/zooms and the inspector resizes. An authored `advance` exposes its
intermediate recorded Execution states. Transitions and journal records remain
distinct: an authored action can also reach an effect without a new record.

`run --ui` and `check --ui` refuse before effects. Editing, simulation
branches and real-I/O experiment debugging remain separate future work.

Record a real run, then inspect its recorded facts without repeating effects:

```sh
printf 'hello\n' | rastok run examples/echo.rkg --record echo.rkr
rastok replay echo.rkr
rastok replay echo.rkr --ui
```

Replay needs neither the source nor the files/processes used by the program.
It reconstructs the recorded state; `simulate` is the command that executes
the DSL against a Lab world. This bounded diagnostic file is not M2's durable
deployment journal. The first slice refuses personal-data persistence and
boot-dependent deployment profiles, never overwrites an existing recording,
and makes no share-safe-export claim. See the
[recording contract](diagnostic-recording.md) for the exact
format, read-only UI and limits.

Compare a simulation with its reviewed causal reference traces, without
changing them:

```sh
rastok simulate examples/echo.rkg --compare examples/traces/echo
```

To intentionally create or replace references, use `--save <dir>` instead
of `--compare <dir>` and review the resulting diff before committing it.
The options are mutually exclusive. With neither option, simulation checks
the Lab expectations without reading or writing reference traces.

See [ui.md](../spec/ui.md) and the
[M1.9 backlog](m1.9-ui-backlog.md) for the presentation and
external visual-qualification boundary.

For hosted execution or simulation, `--deployment <profile.json>` supplies
typed boot values, versioned secret coordinates and the optional privacy
attestation. Secret material itself comes from the selected provider (the M1
provider is the process environment plus a matching `_VERSION` coordinate),
never from a command-line value.

The standalone runner links Process by default. Hosted features add the
selected HTTP/WebSocket Connector implementations through one registry assembly. Pass
`--file-root <dir>` to `rastok run` to add the real File Driver with an
explicit deployment root; that root is neither inferred from the graph nor
part of plan identity.

Pass `--json-logs` to `rastok run` for newline-delimited structured events on
stderr. Boundary outputs remain on stdout. Logging is deliberately lossy under
exporter pressure; the journal-backed report input is not.

For a local paused debugger, build the optional hosted UI:

```sh
cargo run -p rastok --features hosted-ui -- simulate app.rkg --ui
```

Start with a small line-counting simulation: a mocked reader supplies one
header and three data lines, a filter drops the header, and an Operator counts
the remaining lines. This is not a general CSV parser with quoted-newline support.

```sh
cargo run -p rastok --features hosted-ui -- simulate examples/csv-count.rkg --ui
```

The larger executable UI tour combines structured DATA, keyed Retry operations, a nested
Graph, an Operator and a mocked Process boundary:

```sh
cargo run -p rastok --features hosted-ui -- simulate examples/keyed-retry.rkg --ui
```

Add `--ui-port PORT` to request one explicit non-zero loopback port. The CLI
prints the authorized URL and never opens a browser automatically.
The [example catalogue](../examples/README.md) distinguishes executable
programs from historical sketches and future-surface illustrations.

For projects described by OpenAPI 3.0 or 3.1, run
`rastok --resolve-open-api <project-dir>` to discover static HTTP endpoints
and deterministically create or update `openapi.generated.rkg`. The command
never rewrites authored sources; `--check` refuses missing or stale generated
types for CI and LLM-driven workflows.

