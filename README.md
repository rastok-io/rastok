# Rastok

> Integrate AI faster. Keep production calm.

Rastok is an Apache-2.0 runtime and a dedicated domain-specific
language (DSL) for stateful, effectful integration graphs. The DSL is
textual, typed, readable in code review, easy to version in Git, and
constrained enough for an LLM to generate without hiding the result
behind a visual low-code format.

A Rastok graph connects existing services, models, tools, data, and
business processes. It contains three kinds of nodes:

- **Operators** contain pure graph logic. They may update their own
  Node-local state,
  but they cannot read from or write to the external environment.
- **Connectors** are explicit, typed boundaries to the external world:
  APIs, processes, databases, files, brokers, and clocks.
- **Graphs** compose Operators, Connectors, and other Graphs.

The declaration keyword fixes a Node's structural kind. Every Node also has
one effective type — `App`, the ordinary `Graph`/`Operator` default, or an
exact versioned Connector contract — and may carry orthogonal aspects such as
`Loop` or `Saga`. A type says what executes; an aspect adds a separately
validated orchestration protocol. The two are never interchangeable.

This separation makes effects replaceable. In production, Connectors
interact with real systems. In a simulation sandbox called the
**Lab**, controlled handlers replace those external systems while the
same Operators and the same checked graph continue to run.

The Lab controls inputs, time, boundary outcomes, and faults. A
production incident can therefore become a local scenario: replay the
causal path, test a fix, and compare the result without repeating the
original real-world effects.

## Same program, different world

Rastok separates semantic decisions from effect execution:

```text
step(plan, state, event) -> (new_state, effect_intents)

App execution  : effect_intents -> real outcomes
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
  -> journaled, auditable multi-family Lab scenarios with useful golden diffs
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

M1.7 is closed. `Loop` is the first aspect with a closed categorical plan
contract and Registry-free control and termination proofs. Fuel-bounded and
directly ranked Loops cross source, typed HIR and the v14 proof form retained
by current v15, with typed
`in`, aspect-owned
`next`/`expired`, domain terminal outputs and ordinary Graph handlers. Every
completed body application proves exactly one continuation or explicit
termination (`out`, `exit`, another ordinary output, or `stop`). Execution
folds each immediate or delayed root-Graph run from journaled
open/next/admit/close/expired facts; exact exhaustion, independent concurrent
roots, direct-child Call recurrence and direct-body crash reconstruction
execute without a spatial back-edge. The first proof certificate accepts a
direct signed-integer measure that is positive on every recurrent path and
decreases by a known positive literal. The `no_std` validator re-derives that
evidence after decode; unsupported theorem fragments, spatial recovery and
nested-owner Loop gates remain fail-closed. Public Lab scenarios additionally
prove singleton and keyed concurrent runs, ordinary nested-Graph composition,
bounded Process retry with multi-frame data, exact exhaustion, and stale-wake
fencing across instance Closing. See the
[M1.7 exit report](docs/architecture/m1.7-exit-report.md).

M1.8 A0–A4 are closed on categorical package v15. The source front end
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
hashes and all existing causal traces remain unchanged. The ordered work is
tracked in the
[M1.8 backlog](docs/architecture/m1.8-observability-backlog.md).

For hosted execution or simulation, `--deployment <profile.json>` supplies
typed boot values, versioned secret coordinates and the optional privacy
attestation. Secret material itself comes from the selected provider (the M1
provider is the process environment plus a matching `_VERSION` coordinate),
never from a command-line value.

The standalone runner links Process by default. Hosted features add the
selected HTTP/WebSocket families through one registry assembly. Pass
`--file-root <dir>` to `rastok run` to add the real File Driver with an
explicit deployment root; that root is neither inferred from the graph nor
part of plan identity.

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
