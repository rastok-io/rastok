# Rastok

> Integrate AI faster. Keep production calm.

Rastok is an Apache-2.0 runtime and a dedicated domain-specific
language (DSL) for stateful, effectful integration graphs. The DSL is
textual, typed, readable in code review, easy to version in Git, and
constrained enough for an LLM to generate without hiding the result
behind a visual low-code format.

A Rastok graph connects existing services, models, tools, data, and
business processes. In the v16 target agreed on 2026-09-05, a Node is either
a passive Graph scope or an Executor:

- **Graphs** compose a typed boundary, children and static wiring. They own
  instance/lifecycle scope, but no application mailbox, handler or transaction.
- **Operators** are Executors whose transitions are authored as typed DSL
  Dispatch. They may update private `var` state but perform no external I/O.
- **Connectors** are Executors. Each has one exact versioned **Contract**;
  its transition comes from that Contract and typed Driver events. Connectors are the explicit boundaries
  to APIs, processes, databases, files, brokers and time sources.

An **Extension** is attached to one Node and may contribute attributes,
bindings, commands, constraints and, only when necessary, ports. `App` marks
the one public root Graph; `Loop` adds finite temporal recurrence through a
journal-owned manager; `Saga` remains a future Extension. A manager is neither
a Node nor an edge, so temporal recurrence never creates a spatial cycle.
The `rework/v16` branch is migrating to categorical v16. Recursive Dispatch,
case routing, multi-emission and command-based Operator and outbound Call Loops
cross the new package reader. Operator measures and recovery of Operator/Call
mailboxes, joins, lifetimes, Closing and command Loops are tested. Nested
boundary routing is retained and re-derived at decode. Window stop products
drain through live sibling state before the Graph scope closes. Fuel-bounded
Graph Loops with pure Operator bodies now retain one admission before fan-out,
carry exact delivery contexts and recover their commands after causal
quiescence. Graph Calls, joins, nested Loops, annotated recurrence and complete
obligation recovery remain gated.
This is an unqualified cutover, not a finished release: the old Loop
fixtures and categorical goldens still need migration. The
[current evidence and remaining gates](docs/architecture/rework-r4-plan.md)
distinguish implemented paths from the complete target.

This separation makes effects replaceable. In production, Connectors
interact with real systems. In a simulation sandbox called the
**Lab**, controlled Driver scripts replace those external systems while the
same Operators and the same checked graph continue to run.

The Lab controls inputs, time, boundary outcomes, and faults. A
production incident can therefore become a local scenario: replay the
causal path, test a fix, and compare the result without repeating the
original real-world effects.

## Same program, different world

Rastok separates semantic decisions from effect execution:

```text
step(plan, state, event) -> (new_state, effect_intents)

Driver execution: effect_intents -> real outcomes
Lab simulation : effect_intents -> controlled outcomes
```

Boundary outcomes return as typed, journaled events. Rastok does not
pretend that effects are pure. It makes them visible, isolated, and
substitutable.

Readers from functional programming may recognize a deterministic core
surrounded by explicit effect handlers. Rastok does not expose monads
or require a functional language; it preserves the useful engineering
property.

## Status

Rastok is at an early experimental stage. Its specification and
implementation architecture are under active review, and the runtime
does not yet make a production-readiness claim.

The current executable proof is one continuous path:

```text
Rastok DSL source
  -> validated executable plan
  -> deterministic state machine
  -> state-machine execution in a native in-memory VM
  -> mixed real Process and File boundaries selected through one Driver registry
  -> journaled, auditable multi-Contract Lab scenarios with useful golden diffs
  -> optional paused, transaction-steppable hosted inspection
```

Connector contracts have stable semantic ids and inbound/outbound roles.
Current binaries compose Drivers explicitly at build/assembly time; minimal,
embedded and full builds may select different sets without changing the plan
model. A future plugin host can use the same registry boundary, but Rastok does
not yet ship a plugin loader or freeze a native ABI. Contributors follow the
[Connector Driver guide](docs/architecture/connector-drivers.md).

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
[Codex WebSocket gateway](docs/examples/codex-websocket-gateway.rkg) and the
[M1.6 exit report](docs/architecture/m1.6-exit-report.md) record the complete
proof. The single
[release-qualification inventory](docs/architecture/release-qualification.md)
records executed independent HTTP/WebSocket interop and the remaining
ngtcp2/Autobahn qualifications without conflating them with workspace tests.

M1.7 is closed as historical v14/v15 evidence: it proved fuel-bounded and
directly ranked temporal recurrence, logical delay, typed exhaustion,
concurrent runs and stale-wake fencing without a spatial back-edge. The
[M1.7 exit report](docs/architecture/m1.7-exit-report.md) remains the exact
record of that implementation. D-2026-09-05-1 now supersedes its Graph-handler
and port-shaped control model: v16 makes Graph passive, shares transaction
mechanics between Operator and Connector, identifies a run from its exact
opening admission, and represents Next/Again/Done as durable manager commands.
The frozen [R0 algebra](docs/architecture/rework-r0-algebra.md),
[reference traces](docs/architecture/rework-r0-reference-traces.md), and
[R1 host contracts](docs/architecture/rework-r1-host-contracts.md) lead to the
[R2 review handoff](docs/architecture/rework-r2-supersession.md). They are
design documents; no executable v16 claim is made before R3–R8.

M1.8 is closed on categorical package v15. The source front end
recognizes the closed `@sensitive`/`@secret`/`@personal`/`@measure`/`@cost` family,
resolves aliases into canonical schema paths and transports the sparse policy
tables through checked HIR, package decode and Registry-free validation.
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
remain fail-closed. Unannotated semantic
hashes and all existing causal traces remain unchanged. Every causal record
also yields a typed retained observation with exact flow/instance/Call/Session
correlation. `rastok run --json-logs` projects that stream through a bounded
never-block queue; payloads are omitted by default and classified values are
redacted before JSON formatting. Export loss changes neither execution nor
the retained accounting plane. A7 reconstructs a public per-flow
`CostReport` lazily from the same causal records and validated schemas. Token
subset relations prevent cache/reasoning double counting; positive quantities
without a matching deployment price stay `unpriced`. Provider-reported and
pricebook-calculated amounts retain exact decimal values, evidence and
currency, and no cross-currency total exists. Optional Connector/provider/model
price rows live in the deployment profile; only their stable reference and
digest enter the run header, never graph identity or causal bytes. The ordered
work is tracked in the
[M1.8 backlog](docs/architecture/m1.8-observability-backlog.md), and the
[exit report](docs/architecture/m1.8-exit-report.md) owns the identity,
portability and remaining-limit ledger.
The optional OpenTelemetry transport is explicitly deferred until exact
temporal observations exist; no exporter timestamp is presented as commit
time.

The M1.9 UI work is paused until the v16 R9 migration. Its retained design adds
an optional local execution UI without a second engine or causal authority. A
binary built with `hosted-ui` accepts `--ui` on `run` and
`simulate`, binds an OS-selected loopback port by default, prints a
capability-bearing URL and starts paused before the first semantic
transaction. Pausing freezes hosted logical time, so inspection cannot consume
timeout, lifetime or Loop-delay budgets. The page separates lifecycle facts,
Lab actions and port transactions, retains the complete in-memory causal
history after completion, and presents a compound topology, frame-owned ports,
separate `META`/`DATA`, logical instances, Calls/Sessions, structured redacted
state and the exact `CostReport`. Smooth cubic routes follow deterministic
crossing-reduced lanes; `≡ n` badges show only current mailbox occupancy and
open the exact redacted FIFO; labels avoid the routed pipes, and each retained
`B<n>` boundary animates the exact envelope movement or lifecycle owner. The
current v15-only dotted control layer explains historical Graph-handler
emissions; R9 removes that layer instead of inventing an equivalent v16 port.
`⧉ n` opens the current logical instances. The
canvas pans and zooms, its inspector is resizable, and Play presents those same
boundaries at one or four events per second.
`A<n>` identifies scenario actions and `R<n>` causal records. Selecting a Node
or Graph filters those linked views without truncating the retained authorities.
The timeline can inspect exact prior state while a real run continues; a
simulation may deterministically replay a prior boundary into a new immutable
branch and continue from there.
Before a checked simulation `inject`, its typed redacted `DATA` is visible
read-only; inputs remain authored in the `.rkg` scenario and are still admitted
through the ordinary immutable-envelope path. Non-UI execution, plan identity
and causal records are unchanged. See
[`ui.md`](docs/spec/ui.md) and the
[M1.9 backlog](docs/architecture/m1.9-ui-backlog.md).

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

For a local paused debugger, build the optional hosted UI and use either
execution mode:

```sh
cargo run -p rastok --features hosted-ui -- run app.rkg --ui
cargo run -p rastok --features hosted-ui -- simulate app.rkg --ui
```

The executable UI tour combines structured DATA, keyed Loop retries, a nested
Graph, an Operator and a mocked Process boundary:

```sh
cargo run -p rastok --features hosted-ui -- simulate examples/keyed-retry.rkg --ui
```

Add `--ui-port PORT` to request one explicit non-zero loopback port. The CLI
prints the authorized URL and never opens a browser automatically.

For projects described by OpenAPI 3.0 or 3.1, run
`rastok --resolve-open-api <project-dir>` to discover static HTTP endpoints
and deterministically create or update `openapi.generated.rkg`. The command
never rewrites authored sources; `--check` refuses missing or stale generated
types for CI and LLM-driven workflows.

## Documentation authority

The repository keeps intent, contract, design, and evidence distinct:

```text
WHITEPAPER.md             conceptual intent
        |
        v
docs/spec/                normative language and runtime contract
        |
        v
ARCHITECTURE.md
docs/architecture/        implementation design
        |
        v
crates/ and tests         implementation and executable evidence
```

Architecture and code do not silently override the specification. A
semantic change updates the normative rule, affected architecture,
implementation, and executable evidence together.

## Repository

The stable top-level structure is:

```text
rastok/
├── crates/                 Rust implementation workspace
├── docs/
│   ├── spec/               normative language and runtime contract
│   ├── architecture/       implementation views and diagrams
│   └── examples/           graphs and reference simulations
├── tools/                  repository development tools
├── AGENTS.md               engineering rules for humans and agents
├── ARCHITECTURE.md         canonical implementation blueprint
├── CODE_OF_CONDUCT.md      expected project conduct
├── CONTRIBUTING.md         contribution and review policy
├── GOVERNANCE.md           project decision model
├── SECURITY.md             vulnerability reporting policy
├── ROADMAP.md              current evidence gates
└── WHITEPAPER.md           conceptual model and statement of intent
```

Compiler and runtime crates stay together in this workspace. New crate
boundaries are introduced only when the implementation justifies them;
architecture documents do not pre-create empty subsystems.

`LICENSE`, `OPEN_SOURCE_COMMITMENT.md`, and `TRADEMARKS.md` define the
project's licensing and identity commitments.

## License

Rastok is licensed under the Apache License 2.0. The Rastok name
and official artifacts are covered separately by the trademark policy.
