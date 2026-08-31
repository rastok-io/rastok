# Rastok architecture

> Implementation blueprint — non-normative and under review.

This document explains how the current Rastok specification becomes an
executable system. It defines implementation boundaries, dependency direction,
and the current executable foundation. It does not add
language or runtime semantics.

## Authority and change discipline

Repository knowledge flows in one direction:

```text
WHITEPAPER.md             product intent and conceptual model
        ↓
docs/spec/                normative language and runtime contract
        ↓
ARCHITECTURE.md
docs/architecture/        implementation design
        ↓
crates/ and tests         implementation and executable evidence
```

If architecture or code disagrees with the specification, the
specification wins. A semantic change must update the specification,
architecture, implementation, and verification evidence together; an
architecture document must never silently redefine the contract.

## Current executable foundation

The architecture has grown from the first mechanical proof without creating a
second engine:

```text
.rkg source
  → parsed and checked program
  → categorical validated plan with logical instances and resolved port types
  → synchronous one-worker execution over a causal journal
  → exact-id/role binding of Process, File and four live network Driver families
  → deterministic multi-family Lab scenarios and useful golden diffs
  → optional loopback UI over the same transaction/journal/accounting seams
```

The original proof remains in
[`docs/architecture/m0-exit-report.md`](docs/architecture/m0-exit-report.md);
the current state and evidence are indexed by
[`docs/architecture/m1-slice.md`](docs/architecture/m1-slice.md).
[`ROADMAP.md`](ROADMAP.md) remains the source for milestone gates.

The current slice does not require clustering, a durable disk journal, a live
Saga manager, a complete Connector catalogue, a plugin loader, a remote web
console, or enterprise deployment features. The local optional `--ui` surface
is deliberately not a remote administration plane. Those capabilities
must grow from demonstrated boundaries rather than be represented by
empty crates.

## Architectural shape

Rastok uses a deterministic semantic core surrounded by explicit effect
boundaries:

```text
                         ┌──────────────────────────────┐
event + state + plan ──▶ │ deterministic semantic step │
                         └──────────────┬───────────────┘
                                        │ effect intent
                            ┌───────────┴───────────┐
                            │                       │
                    App handler              Lab handler
                    real boundary       controlled boundary
                            │                       │
                            └───────────┬───────────┘
                                        │ typed outcome event
                                        ▼
                                      journal
```

The App and Lab interpret the same checked plan. The semantic core does
not read the wall clock, environment, randomness, filesystem, or
network implicitly. Such observations enter through typed inputs or
effect outcomes and can therefore be recorded and replayed.

## Logical responsibilities

These are design responsibilities inside the bootstrap crate, not a
promise of one crate per box:

| Responsibility | Owns | Must not own |
|---|---|---|
| Source front end | parsing `.rkg`, source spans, syntax diagnostics | runtime effects |
| Semantic checker | names, types, graph validity, aspect constraints | host I/O |
| Planner and canonicalizer | immutable executable plan, stable identity and ordering | live execution state |
| Runtime core | state transitions, scheduling decisions, effect intents | direct host calls |
| Journal boundary | ordered typed inputs and outcomes | hidden semantic decisions |
| Runtime Connector binding | exact-id and role association before VM/journal construction | compiler catalogue or first-registration-wins selection |
| Process, File and network Drivers | physical external interactions behind typed Contracts; flat family modules own binding and private `network` owns sockets/TLS/protocol engines | general runtime semantics, compiler schema projection or journal authority |
| Project schema tooling | strict OpenAPI discovery/projection and deterministic generated Rastok source | a second runtime type system or authored-source mutation |
| Lab runner | controlled handlers, virtual inputs, scripted faults | a second execution engine |
| Hosted execution UI | paused transaction controls and disposable projections of validated topology, VM inspection, journal observations and accounting | execution authority, durable history, remote administration or unredacted classified data |
| Diagnostics and golden diff | stable human-facing evidence | mutation of execution |

The component-level blueprint refining this table — owned data forms,
interfaces, dependency direction, and M0 status per component — is
[`docs/architecture/components.md`](docs/architecture/components.md).
The working map from specifications to these responsibilities is in
[`docs/architecture/spec-to-code.md`](docs/architecture/spec-to-code.md).

## Dependency direction

Dependencies point toward semantic data and pure decisions:

```text
entry points
  ├─ App runner ─┐
  └─ Lab runner ─┴─▶ effect boundary ─▶ runtime core
                                             ▲
source front end ─▶ checker ─▶ planner ──────┘

Connector Drivers depend on boundary contracts;
the semantic core never depends on concrete Driver implementations.
```

Shared types should be owned by the layer that defines their invariant,
not collected in a generic utility module. Queued or potentially
durable values are owned data. Errors remain typed until the outermost
presentation boundary.

## Boundary rules

1. **One engine, two interpretations.** App and Lab do not fork the
   planner or runtime semantics.
2. **Effects are values before they are actions.** The core emits an
   intent; a handler produces a typed outcome.
3. **Nondeterminism is explicit.** Time, randomness, environment, and
   external responses cannot enter through ambient APIs.
4. **Identity is canonical.** Stable identifiers and hashes derive only
   from the canonical forms defined by the specification.
5. **Recovery is journal-relative.** In-memory M0 may use a simple
   journal, but its API must not erase the distinction between logical
   events and a future physical layout.
6. **Abstractions earn their place.** A trait, generic parameter, crate,
   or dependency needs a present boundary or test seam. Repetition is
   preferable to a false abstraction.
7. **Safe Rust by default.** Unsafe code requires a demonstrated need,
   a documented invariant, and focused tests.

## Crate policy

The std/compiler/Driver layer stays in the `rastok` crate while its real
dependency graph is being discovered. A responsibility becomes a separate crate only
when at least one concrete pressure exists:

- independent reuse by another executable;
- a dependency that must not leak into the semantic core;
- a compile-time or ownership boundary that improves correctness;
- a separately testable protocol with a stable public surface.

Splitting by architectural noun alone is not sufficient. Compiler and
runtime crates, once justified, remain in this repository so that
specification, implementation, and conformance changes can be atomic.

The first split happened at M0.5 under exactly this rule:
`rastok-core` (no_std + alloc — vocabulary, envelope, expressions, plan,
journal, boundary, machine) separated from the std layer when the
"dependency that must not leak" pressure became concrete — the OS
must never enter the kernel, and a bare-metal target must be able to
run the same engine. The frontier is machine-checked: CI builds
`rastok-core` for Cortex-M33, Cortex-M3 and RV32IMAC on every push, and the std crate
re-exports the kernel so callers see one surface.

M1.3 deliberately revisits one name and ownership boundary inside that
crate: Rastok's canonical logical `DataType`/`Value` moves to the core,
while the CEL-compatible machinery becomes the `expr` consumer of that
domain. JSON Schema, Avro and Arrow adapters stay in the std/boundary
layer; their library types never leak into the no_std kernel. See the
[M1.3 backlog](docs/architecture/m1.3-type-system-backlog.md).

M1.6 H4.1 extends that same core algebra with the open canonical
`media_type` scalar. OpenAPI remains a hosted compiler/tool adapter:
`serde_json`/`serde_yaml_ng` and filesystem discovery stay in `rastok`, while
only resolved `DataType` trees and categorical plan data cross toward the
core. Connector modules are flat by contract family; a shared module is named
for its real responsibility (`websocket_protocol` or private `network`) rather
than becoming a generic utility bucket.

M1.6 H7 closes the hosted boundary without widening the core interface. One
feature-selected `hosted_registry` composes Process, optional File and the
linked HTTP/WebSocket families. HttpClient/HttpServer remain Calls;
WebSocketClient/WebSocketServer use the generic Session fold over H1 Upgrade
and H2/H3 Extended CONNECT. Tokio and every protocol-library type remain
private to `rastok::connectors::network`; the same core still builds for the
three established no-std targets.

M1.7 keeps temporal recursion on that same deterministic boundary. The
`Loop` manager consumes the categorical aspect contract introduced by v14 and
retained by v15, then folds ordinary
journal facts; optional dated re-entry uses the generic scheduler, not a
thread sleep or a spatial cycle. The compiler and Registry-free core prove a
finite control verdict plus either positive fuel or the admitted integer
ranking certificate before the VM can execute the aspect. Public Lab and
Cortex-M33 evidence are recorded in the
[M1.7 exit report](docs/architecture/m1.7-exit-report.md).

M1.9 keeps interactive execution outside that boundary. The optional
`hosted-ui` feature embeds one loopback HTTP/SSE page and a physical controller
whose `Step` command invokes exactly one existing VM transaction. It derives
history, structured redacted values and `CostReport` from existing read seams;
capabilities, controller ownership, browser revisions and port numbers enter
no package, semantic or causal identity. Reduced and embedded builds omit the
server and assets entirely.

## Views

- [System context](docs/architecture/01-system-context.svg) —
  people and external systems around Rastok.
- [M0 container view](docs/architecture/02-rastok-container.svg) —
  the executable/deployment boundary for the first slice.
- [M0 component view](docs/architecture/03-rastok-components.svg) —
  logical responsibilities inside the executable.
- [App/Lab effect cycle](docs/architecture/04-app-lab-effect-cycle.svg)
  — the shared semantic step and its two handlers.
- [Archived pre-build module and type blueprint](docs/architecture/05-rastok-packages.svg)
  — the M0 design artifact, retaining superseded names as history rather than
  current API.
- [Module overview](docs/architecture/06-rastok-modules-overview.svg)
  — the bird's-eye companion: every module and type with a one-line
  description, module-level dependencies only.

The adjacent `.puml` files are the editable diagram sources. Rendering
instructions live in [`tools/diagrams/`](tools/diagrams/README.md).

## Decisions intentionally left open

The blueprint does not yet select:

- the physical durable-journal layout and recovery deployment;
- a serialized plugin manifest syntax, loader, trust policy or native ABI;
- a public embedding API beyond the current library surface.

Each decision should be made at the smallest point where executable
evidence exposes its real constraints, and then recorded here or in a
focused architecture note.
