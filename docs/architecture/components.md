# Component blueprint

> Implementation blueprint — non-normative and under review.

This document refines the responsibility table of
[`ARCHITECTURE.md`](../../ARCHITECTURE.md) to the component level: the
data forms that flow through the system, the Rust modules and types
that own them, the dependency direction between modules, and what M0
builds. It adds no language or runtime semantics; on any conflict,
[`docs/spec/`](../spec/README.md) wins. The
[C4 component view](03-rastok-components.svg) remains the coarse visual;
the [module and type blueprint](05-rastok-packages.svg) draws the full
type-level projection of this inventory.

## How to read this

- A **component** is a responsibility with an owned data form and a
  typed interface. Components live as modules — or private types —
  inside the bootstrap crate; the crate policy of `ARCHITECTURE.md`
  governs promotion.
- **Interfaces are data.** Modules hand each other the immutable forms
  of §1. No module reaches into another's internals through callbacks
  or shared mutable state.
- **Status legend** — `M0`: built by the first slice, possibly as a
  reduced subset; `next`: not built in M0, but M0's APIs must already
  accommodate it; `guarded`: no implementation, yet reachable syntax
  produces an explicit diagnostic, never an accident; `later`: absent,
  no stub.

## 1. The data spine

The system is a chain of owned, immutable forms. Every module below is
defined by the form it produces or consumes.

```text
.rkg source bytes
  │ parse
  ▼
syntax tree (+ source spans)
  │ assemble, desugar, resolve, type
  ▼
checked graph ────────────────▶ canonical bytes ─▶ app::Hash
  │ lower                                          (the identity)
  ▼
executable App (tables + expression trees — data, not code)
  │ interpret
  ▼
envelopes, effect intents, typed outcomes
  │ record
  ▼
typed journal records (INBOUND / TRANSACTION / OUTBOUND)
  │ project
  ▼
canonical record projection ─▶ golden compare
```

| Form | Produced by | Consumed by | Load-bearing invariant |
|---|---|---|---|
| syntax tree + spans | `compiler` (parse) | check passes, `diag` | faithful to the text, sugar intact; may contain error nodes; an error node is never lowered |
| checked graph | `compiler` (check) | canonicalize, lower | desugared (one way to say each thing), names resolved, fully typed; spans live in side tables, never in the canonical form |
| canonical bytes, `app::Hash` | `compiler` (canonicalize) | `app::Manifest`, golden headers | byte-stable across formatting, file order, and sugar spelling ([identity.md §1](../spec/identity.md)) |
| `app::App` | `compiler` (lower) | `vm`, `lab` | immutable, serializable data — tables plus `cel::Tree`s; no closures, no host references; names resolved to `Slot`s, symbols kept in a side table |
| `world::Intent` (open OUTBOUND) | `vm` | `world::Partner` impls | journaled before any handler runs ([delivery-semantics.md §4](../spec/delivery-semantics.md)) |
| `world::Outcome` | `connectors` or `lab::Mock` | `vm`, `journal` | one of the sealed closure states; a late reply is an observation, never a causal fact |
| `journal::Record` | `vm` + world impls | recovery, `journal::Golden` | causal core and observation annotations kept separable ([identity.md §3](../spec/identity.md)) |
| `diag::Diagnostic` | every phase | entry points | stable code, spans, structured fields; accumulated values — never prints, never panics |
| golden projection | `journal::Golden` | evidence compare | deterministic bytes: same scope, same bytes ([simulation-semantics.md §6](../spec/simulation-semantics.md)) |

## 2. The module map

```text
crates/rastok/src/
├── node.rs         # structural vocabulary : Path, Kind, Port, EffectClass
├── envelope.rs     # Envelope, Meta, MetaSys, Id, Value, Trigger
├── diag.rs         # Diagnostic, Severity, Phase, Span, Sink
├── cel.rs          # the isolated expression chain — depends on nothing here
├── compiler.rs     # parse → assemble → check → lower → canonicalize
├── app.rs          # App, Handler, Slot, GuestProgram, Hash, Version, Manifest
├── world.rs        # Ingress, Partner, Clock, Supervisor + Intent, Outcome
├── vm.rs           # THE engine — run and sim ; GuestExecutor ; BootEnv (next)
├── guests.rs       # jq / sql engines (guarded) — the pure twin of connectors
├── journal.rs      # Journal trait, Record, ids, Golden, fold (next)
├── connectors.rs   # one file per connector ; Process first
├── lab.rs          # Runner + Fixture / Mock / VirtualClock / Faults
└── main.rs         # thin bin : check | run | sim
```

Naming rules, fixed for the whole crate:

1. **Spec-first names.** A module carries the name of the specification
   concept that owns its invariant when one exists (`envelope`,
   `journal`, `lab`, `node`, `app`); purely internal machinery takes
   plain engineering names (`compiler`, `vm`, `cel`).
2. **Unprefixed types, qualified paths.** `cel::Tree`, `app::Hash`,
   `world::Intent` — never `CelTree`. The `x::X` stutter is idiomatic
   and assumed: `app::App`, `vm::Vm`, `journal::Journal`.
3. **Blessed abbreviations only**, used consistently everywhere:
   `diag`, `vm`. Nothing ad hoc.
4. **Flat, depth ≤ 2.** A directory exists for exactly two reasons — a
   catalogue (one file per item: `connectors/`, `guests/`) or an
   overflow split behind an unchanged facade (`compiler/`). Never
   created in advance.
5. **One engine.** There is no simulator component: `rastok sim` is the
   same `vm` wired to the lab's implementations of the four `world`
   traits.

## 3. Component inventory

Columns: what the component owns, its interface stated as data in →
data out, the governing spec, and M0 status.

### 3.1 The compiler (`compiler`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **Parser** | grammar surface, spans, error recovery | source bytes → syntax tree + diagnostics | [grammar.md](../spec/grammar.md) | M0 subset |
| **Unit assembly** | the deployed unit: multi-file resolution, lab-fragment merge, exactly-one-App, wiring by name, no shadowing | syntax trees → one resolved `Unit` | [grammar.md](../spec/grammar.md) scope rules; [simulation-semantics.md §2](../spec/simulation-semantics.md) | M0 (single unit) |
| **Desugarer** | the finite sugar table — afterwards there is one way to say each thing | resolved unit → core-form graph | [grammar.md](../spec/grammar.md) | M0 |
| **Type checker** | envelope and data types, mandatory err sets, variant narrowing, aspect and effect admission | core-form graph → `CheckedGraph` + diagnostics | [grammar.md](../spec/grammar.md); [aspects.md](../spec/aspects.md); [effects-and-sagas.md](../spec/effects-and-sagas.md) | M0 subset |
| **Canonicalizer** | the canonical byte form of the checked graph; the semantic hash | `CheckedGraph` → canonical bytes + `app::Hash` | [identity.md §1](../spec/identity.md) | M0 |
| **Lowering** | node/edge/partition/handler tables, expression trees, `Slot` resolution | `CheckedGraph` → `app::App` | [identity.md](../spec/identity.md); [execution-model.md](../spec/execution-model.md) | M0 |
| **Guest frontends** | `GuestFrontend` trait — check an opaque guest body against its signature; the host parser never reads it | guest bytes → `app::GuestProgram` | [stdlib.md §5](../spec/stdlib.md) | guarded |

Two compiler rules with consequences elsewhere:

1. **The CEL chain is isolable** (§3.2): the checker hands it a
   *signature*, never an envelope or node.
2. **Desugaring is provable.** Because identity hashes the desugared
   checked graph, "sugared and long forms hash identically" is a free,
   mechanical test for every sugar in the table.

### 3.2 The CEL chain (`cel`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`cel`** | the scalar expression language: compile, evaluate, const-evaluate over canonical typed trees | (text, `Signature`) → `Tree`; (`Tree`, `Bindings`) → `Value`; (`Tree`, boot constants) → constant | [grammar.md](../spec/grammar.md) CEL positions; [execution-model.md §5](../spec/execution-model.md) (no ambient time) | M0 subset |

`cel` never sees an envelope, node, or partition, and owns its own
`cel::Diagnostic` (the compiler adapts it into `diag::Diagnostic`).
The *signature* — the typed names visible at one expression position —
is the entire interface, which keeps the chain independently
conformance-testable against the upstream CEL specification. It is the
first candidate for promotion to its own crate.

### 3.3 The executable App (`app`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`app::App`** | the executable form — the tables and expression trees the VM interprets, and what ships to other hosts | produced by `compiler`, consumed by `vm` and `lab` | [identity.md](../spec/identity.md); [execution-model.md](../spec/execution-model.md) | M0 |
| **`app::Manifest`** | the append-only ledger `Hash → Version`; a version is never renumbered, never reused | `app::Hash` → `app::Version` | [identity.md §2](../spec/identity.md) | M0 (minimal) |

The App ships unresolved: `env()` and `secret()` stay expression nodes
in the tables, and boot binds them in memory — the same artifact runs
on a laptop and in production.

### 3.4 The VM (`vm`)

Deterministic by construction: no I/O, no clock reads, no randomness.
Everything nondeterministic enters through the `world` traits (§3.5).
The same engine runs `rastok run` and `rastok sim`.

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **Scheduler** | run-to-completion firing; the single dated-event queue; virtual-time `advance`; drain-to-stable | App + ordered events → transaction firings | [execution-model.md §1, §5, §8](../spec/execution-model.md) | M0 (single worker) |
| **Mailboxes** | per-key FIFO order, arrival-order capture (the race outcome is journaled), bounded capacity | envelopes → ordered per-key delivery | [execution-model.md §2–3](../spec/execution-model.md) | M0 |
| **TransactionExecutor** | the atomic node transaction: consumed input, state delta, emissions, and records commit together; scratch isolation while a handler runs | firing → transaction record + emissions | [execution-model.md §1](../spec/execution-model.md) | M0 |
| **EmissionBuffer** | per-port emission identity in text order (`emission_index`) | handler body → ordered emissions | [execution-model.md §4](../spec/execution-model.md) | M0 |
| **Error routing** | typed err values; lexical bubbling to `app.err`; nothing silent | err emissions → routed envelopes | [delivery-semantics.md §5](../spec/delivery-semantics.md) | M0 |
| **`GuestExecutor`** | the trait guest engines implement: run(program, value, budget) — pure, deterministic, budgeted, inside the node transaction | `app::GuestProgram` → value or typed error | [stdlib.md §5](../spec/stdlib.md) | guarded |
| **`BootEnv`** | `env()` / `secret()` resolution and constant positions, evaluated at boot; a missing or ill-typed binding is a typed refusal, never a default; `Secret<T>` is redacted by type | App + deployment profile → bound constants | [grammar.md](../spec/grammar.md) initializer positions | next |

### 3.5 The world seam (`world`) — the four host interfaces

Every nondeterministic thing crosses exactly one of these four traits,
and what crossed is recorded — the journal's *world-half* is the record
of it. The lab substitutes each one without touching the VM: in run,
the world plays itself; in sim, the lab plays the world.

| Interface | Owns | Real side | Lab side | Spec | M0 |
|---|---|---|---|---|---|
| **`Ingress`** | birth of inbound envelopes and the inbound sequence (`IngressAuthority` stays opaque); bounded, backpressure translated outward | listeners | `lab::Fixture` | [envelope.md §2](../spec/envelope.md); [delivery-semantics.md §3](../spec/delivery-semantics.md) | M0 (minimal) |
| **`Partner`** | executing open `Intent`s; typed `Outcome`s; retry deduplication on the stable business key | `connectors/*` | `lab::Mock` | [delivery-semantics.md §4](../spec/delivery-semantics.md) | M0 (Process only) |
| **`Clock`** | the only source of time, delivered as dated events | wall clock | `lab::VirtualClock` | [execution-model.md §5, §8](../spec/execution-model.md) | M0 (virtual required) |
| **`Supervisor`** | process lifecycle: boot, crash, restart; recovery must never require a clean shutdown | host lifecycle | `lab::Faults` | [delivery-semantics.md §4.4](../spec/delivery-semantics.md); [simulation-semantics.md §5.4](../spec/simulation-semantics.md) | M0 (boot only); faults `next` |

### 3.6 The journal (`journal`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`Journal`** | the typed logical records and their order; `MemJournal` in-memory first | append / ordered read | [delivery-semantics.md §6](../spec/delivery-semantics.md); [envelope.md §5](../spec/envelope.md) | M0 |
| **Id derivation** | derived envelope and record ids — zero per-envelope randomness | recipe inputs → `envelope::Id` | [envelope.md §2–3](../spec/envelope.md) | M0 |
| **`Fold`** | journal → state, dated obligations, open-effect triage; snapshots are derived accelerations, never causal | records → recovered runtime state | [delivery-semantics.md §4.4, §6–7](../spec/delivery-semantics.md) | next |
| **`Golden`** | the canonical record form; the causal-core / annotation split; compare with explicit bless | records → deterministic bytes; two runs → first meaningful divergence | [identity.md §3](../spec/identity.md); [simulation-semantics.md §6](../spec/simulation-semantics.md) | M0 |

### 3.7 Connectors and guests (`connectors`, `guests`)

The effectful and the pure halves of extension, on mirrored seams: a
`connectors/*` adapter implements `world::Partner`; a `guests/*` engine
implements `vm::GuestExecutor` (and `compiler::GuestFrontend`). A guest
program with I/O belongs behind a Connector, whatever its execution
format.

| Component | Owns | Spec | M0 |
|---|---|---|---|
| **`connectors::Process`** | the first real partner: spawn, observe, typed exit | [stdlib.md §3.3](../spec/stdlib.md) | M0 subset |
| **Catalogue remainder** (HttpServer, HttpClient, Clock, Calendar, Memory) | declared contracts only as far as the checker needs them | [stdlib.md §3](../spec/stdlib.md) | guarded |
| **`guests::Jq`, `guests::Sql`** | pure guest engines under the six-clause contract | [stdlib.md §5](../spec/stdlib.md) | guarded |

### 3.8 The lab (`lab`)

An adapter set plus a driver — never a second engine.

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`Runner`** | driving the VM through a scenario: inject, advance, expect, golden; observation windows; the three expectation quantifiers | scenario → run + verdicts | [simulation-semantics.md §2, §5](../spec/simulation-semantics.md) | M0 |
| **World implementations** | `Fixture` (Ingress), `Mock` (Partner — a mocked call still writes OUTBOUND), `VirtualClock` (Clock), `Faults` (Supervisor, `next`) | scripted world behind the same four traits | [simulation-semantics.md §2, §5](../spec/simulation-semantics.md) | M0 |
| **Coverage** | the statically enumerable failure-space checklist | App + scenarios → coverage report | [simulation-semantics.md §7](../spec/simulation-semantics.md) | later |
| **Unsupported surfaces** | `integration`, `theorem`, `experiment`: parse, then refuse explicitly | reachable syntax → `unsupported` diagnostic | [simulation-semantics.md §10–11](../spec/simulation-semantics.md) | guarded |

Scenarios are parsed by the same `compiler` front end as graphs — the
simulation grammar is the graph grammar mirrored — so the lab owns no
second parser and no second engine.

### 3.9 Cross-cutting (`node`, `diag`, `main`)

| Component | Owns | Spec | M0 |
|---|---|---|---|
| **`node`** | the structural vocabulary every layer shares: `Path` (enters the id recipes), `Kind`, `Port`, `EffectClass` | [grammar.md](../spec/grammar.md) vocabulary; [effects-and-sagas.md](../spec/effects-and-sagas.md) | M0 |
| **`diag`** | stable codes, severity, phase, multi-span, structured fields; a check ships with its code, its negative test, and its registry row in the same change | [diagnostics.md](../spec/diagnostics.md) | M0 |
| **`main`** | the `check` / `run` / `sim` verbs, golden bless, rendering; owns nothing semantic | — | M0 |

## 4. Dependency direction

```text
main (bin)
   │
   ├──▶ lab (Runner + world impls) ──┐
   │                                 ├──▶ world contracts ──▶ vm
   └──▶ run wiring ──────────────────┘         ▲                │
                                               │                ▼
        connectors ── implement ───────────────┘            journal
        guests ────── implement GuestExecutor ▶ vm
                                                                │
compiler ──▶ app (the executable App) ◀── interprets ────────── vm
    │
    ▼
   cel   (signatures in, trees out — knows nothing else)
```

Rules, in force from the first commit:

1. **`vm`** depends only on `app`, the `node`/`envelope` value types,
   the `world` contracts it emits into, and the `journal` trait — never
   on an adapter, the lab, or the compiler.
2. **`cel`** depends on nothing else in Rastok.
3. **Adapters** (`connectors`, `guests`, the lab's world
   implementations) implement traits; nothing depends on an adapter
   except `main`'s wiring.
4. **`lab`** depends on `compiler`, the `world` contracts, and
   `journal`. It never patches or introspects `vm` internals.
5. **`node`, `envelope`, `diag`** are the base layer: depended on
   everywhere, depending on nothing (except `envelope` on `node`).
6. **`journal::Golden`** reads the journal, not the VM.

Within one crate these rules are held by review, not by Cargo — module
cycles are legal in Rust. Promotion to crates (per the crate policy)
is what will eventually make them mechanical.

## 5. Seams that keep later milestones possible

M0 builds none of the following capabilities, but its APIs must not
foreclose them. These are obligations on M0 code, not features.

- **Logical journal, physical later.** `journal::Journal` exposes typed
  records and their order. Segment files, snapshots-on-disk, and
  retention arrive later behind the same trait
  (`ARCHITECTURE.md`, boundary rule 5).
- **Opaque origin authorities.** `world::IngressAuthority` — and any
  future edge cursor — are opaque types from the start. M0 code never
  assumes the replica component of an id recipe is a constant it may
  inline ([identity.md §3](../spec/identity.md), the INBOUND key
  table). This keeps the multi-host door open without building
  anything behind it.
- **One canonical encoder.** Every canonical form — checked graph,
  App, record projection — uses the same deterministic encoding
  discipline with per-form, extend-only key tables
  ([identity.md §3](../spec/identity.md)). New features add keys; they
  never renumber.
- **Core versus annotation, everywhere.** Every record keeps the split
  between causal core and observation annotations, because golden
  compare is defined on cores only.
- **Refusal is a feature.** An error-node tree is never lowered; a
  reachable-but-unbuilt surface produces its explicit diagnostic; an
  unresolved `env()` refuses to boot. Silence is the only forbidden
  outcome.

## 6. M0 coverage map

The slice stages of [`m0-slice.md`](m0-slice.md), in module terms:

| Slice stage | Exercised |
|---|---|
| Parse | `compiler` (parse), `diag` |
| Check | `compiler` (assemble, desugar, check), `cel` (compile) |
| Plan | `compiler` (canonicalize, lower), `app::Manifest` |
| Step | `vm` (Scheduler, Mailboxes, TransactionExecutor, EmissionBuffer, error routing) |
| Handle | `world::Partner`; `connectors::Process` (App) or `lab::Mock` (Lab) |
| Record | `journal` (records, id derivation) |
| Compare | `journal::Golden`, `main` |

Deliberately without M0 code: `journal::Fold` and `lab::Faults`
(`next` — the record API already carries their inputs), `vm::BootEnv`
(`next`), `guests` and the connector catalogue beyond Process
(`guarded`), coverage (`later`), and live Saga and Loop managers
(deferred per [`spec-to-code.md`](spec-to-code.md); the checker still
validates or explicitly refuses their syntax within the M0 subset).
