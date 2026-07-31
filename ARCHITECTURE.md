# Rastok architecture

> Implementation blueprint — non-normative and under review.

This document explains how the current Rastok specification is intended
to become an executable system. It defines implementation boundaries,
dependency direction, and the first end-to-end slice. It does not add
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

## First mechanical proof

The current milestone is deliberately narrow:

```text
.rkg source
  → parsed and checked program
  → canonical executable plan
  → in-memory single-worker execution
  → one real Process boundary
  → deterministic Lab scenario and useful golden diff
```

The detailed slice and its evidence are defined in
[`docs/architecture/m0-slice.md`](docs/architecture/m0-slice.md).
[`ROADMAP.md`](ROADMAP.md) remains the source for milestone gates.

The first slice does not require clustering, a durable disk journal,
live Saga or Loop managers, a complete connector catalogue, a registry,
a web console, or enterprise deployment features. Those capabilities
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
| Effect dispatcher | routing an intent to the selected handler | connector-specific policy |
| Process adapter | the first real external Process interaction | general runtime semantics |
| Lab runner | controlled handlers, virtual inputs, scripted faults | a second execution engine |
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

connector adapters depend on boundary contracts;
the semantic core never depends on connector implementations.
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

M0 stays in the `rastok` bootstrap crate while the real dependency graph
is being discovered. A responsibility becomes a separate crate only
when at least one concrete pressure exists:

- independent reuse by another executable;
- a dependency that must not leak into the semantic core;
- a compile-time or ownership boundary that improves correctness;
- a separately testable protocol with a stable public surface.

Splitting by architectural noun alone is not sufficient. Compiler and
runtime crates, once justified, remain in this repository so that
specification, implementation, and conformance changes can be atomic.

## Views

- [System context](docs/architecture/01-system-context.svg) —
  people and external systems around Rastok.
- [M0 container view](docs/architecture/02-rastok-container.svg) —
  the executable/deployment boundary for the first slice.
- [M0 component view](docs/architecture/03-rastok-components.svg) —
  logical responsibilities inside the executable.
- [App/Lab effect cycle](docs/architecture/04-app-lab-effect-cycle.svg)
  — the shared semantic step and its two handlers.
- [Module and type blueprint](docs/architecture/05-rastok-packages.svg)
  — Rust modules, key types and traits, and build artifacts
  (indicative, not a public API).
- [Module overview](docs/architecture/06-rastok-modules-overview.svg)
  — the bird's-eye companion: every module and type with a one-line
  description, module-level dependencies only.

The adjacent `.puml` files are the editable diagram sources. Rendering
instructions live in [`tools/diagrams/`](tools/diagrams/README.md).

## Decisions intentionally left open

The blueprint does not yet select:

- the parser implementation or concrete syntax-tree library;
- the final internal module layout;
- the physical journal format;
- the Process transport and wire protocol;
- a public embedding API.

Each decision should be made at the smallest point where executable
evidence exposes its real constraints, and then recorded here or in a
focused architecture note.
