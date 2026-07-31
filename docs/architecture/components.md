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
type-level projection of this inventory, and the
[module overview](06-rastok-modules-overview.svg) is its bird's-eye
companion.

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
HIR ──────────────────────────▶ canonical bytes ─▶ plan::SemanticHash
  │ lower                                          (the identity)
  ▼
executable plan (tables + expression trees — data, not code)
  │ interpret
  ▼
envelopes, effect intents, response events
  │ record (runtime mechanics only)
  ▼
typed journal records (INBOUND / TRANSACTION / OUTBOUND)
  │ project
  ▼
canonical record projection ─▶ golden compare
```

| Form | Produced by | Consumed by | Load-bearing invariant |
|---|---|---|---|
| syntax tree + spans | `compiler` (parse) | check passes, `diagnostics` | faithful to the text, sugar intact; may contain error nodes; an error node is never lowered |
| HIR | `compiler` (check) | canonicalize, lower | desugared, name-resolved, typed — the hash input ([identity.md §1](../spec/identity.md)); spans live in side tables; **the entire lab is excluded from the hash** |
| canonical bytes, `plan::SemanticHash` | `compiler` (canonicalize) | `plan::Manifest`, golden headers | byte-stable across formatting, file order, and sugar spelling ([identity.md §1](../spec/identity.md)) |
| `plan::ExecutionPlan` | `compiler` (lower) | `runtime`, `lab` | immutable, serializable data — tables plus `cel::Tree`s; no closures, no host references; names resolved to `Slot`s |
| `boundary::Intent` (open OUTBOUND) | `runtime` | dispatcher → `boundary::Partner` impls | journaled before any handler runs, carrying the canonical call fields — emitting id, connector path, request digest, optional idempotency key ([delivery-semantics.md §4](../spec/delivery-semantics.md); [identity.md §3](../spec/identity.md)) |
| `boundary::Response` | `connectors` or `lab::PartnerMock` | `runtime` (as an input event) | returns with the internal `CallRef`, correlated by the *connector's own protocol correlator* — never by the business key; a partner only ever answers: it never decides a timeout and never mints `OutcomeUnknown` |
| `journal::Record` | **runtime mechanics only** | recovery, `journal::GoldenProjection` | adapters and mocks supply events, never records; causal core and observation annotations kept separable ([identity.md §3](../spec/identity.md)) |
| `diagnostics::Diagnostic` | every phase | entry points | stable code, spans, structured fields; accumulated values — never prints, never panics |
| golden projection | `journal::GoldenProjection` | evidence compare | deterministic bytes: same scope, same bytes ([simulation-semantics.md §6](../spec/simulation-semantics.md)) |

## 2. The module map

```text
crates/rastok/src/
├── node.rs           # vocabulary : Path, Kind, Port, Effect, Behavior, Trigger
├── envelope.rs       # Envelope, Meta, MetaSys, Id, Data, Encoding, Value
├── diagnostics.rs    # Diagnostic, Severity, Phase, Span, Bag
├── cel.rs            # the isolated expression chain — depends on nothing here
├── compiler.rs       # parse → assemble → check → lower → canonicalize
├── plan.rs           # ExecutionPlan, Handler, Slot, GuestProgram, SemanticHash
├── stdlib.rs         # declarative Connector/guest contracts (the checker's view)
├── boundary.rs       # Ingress, Partner, Clock, Supervisor + Dispatcher, Intent
├── runtime.rs        # Vm (boot, step, drain) ; GuestExecutor ; BootBindings (next)
├── guest_engines.rs  # jq / sql engines (guarded) — the pure twin of connectors
├── journal.rs        # Journal trait, records, ids, GoldenProjection, RecoveryFold
├── connectors.rs     # one file per connector ; Process first
├── lab.rs            # ScenarioRunner + Fixture / PartnerMock / VirtualClock
└── main.rs           # thin bin : check | run | sim
```

Naming rules — **M0 working conventions**, revisited at crate
promotion (the final layout stays open per `ARCHITECTURE.md`):

1. **Spec-first names.** A module carries the name of the concept that
   owns its invariant in the normative corpus (`envelope`, `journal`,
   `lab`, `node`, `stdlib`) or in the whitepaper (`plan` — the
   executable plan); purely internal machinery takes plain engineering
   names (`compiler`, `runtime`, `boundary`, `cel`).
2. **Explicit English over abbreviations.** A newcomer must understand
   a name without the project's internal vocabulary: `diagnostics`,
   `guest_engines`, `ScenarioRunner`, `RecoveryFold`. `cel` is a
   proper noun, not an abbreviation. No blessed abbreviations remain.
3. **Qualified paths give context; precision beats brevity.** A type
   never repeats its module name (`cel::Tree`, never `CelTree`), but
   keeps a precise compound when the bare word would be too generic
   (`lab::ScenarioRunner`, `journal::GoldenProjection`) and keeps the
   corpus term even when it overlaps the module word
   (`plan::ExecutionPlan`, `plan::SemanticHash`). The `x::X` stutter
   stays idiomatic where the type *is* the module's whole concept:
   `journal::Journal`, `envelope::Envelope`.
4. **Flat, depth ≤ 2.** A directory exists for exactly two reasons — a
   catalogue (one file per item: `connectors/`, `guest_engines/`) or an
   overflow split behind an unchanged facade (`compiler/`). Never
   created in advance.
5. **One engine.** There is no simulator component: `rastok sim` is the
   same `runtime` wired to the lab's implementations of the four `boundary`
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
| **Type checker** | envelope and data types, mandatory err sets, variant narrowing, aspect admission; node declarations as an ADT so illegal states are unrepresentable | core-form graph → `Hir` + diagnostics | [grammar.md](../spec/grammar.md); [aspects.md](../spec/aspects.md); [effects-and-sagas.md](../spec/effects-and-sagas.md) | M0 subset |
| **Canonicalizer** | the canonical byte form of the HIR; the semantic hash | `Hir` → canonical bytes + `plan::SemanticHash` | [identity.md §1](../spec/identity.md) | M0 |
| **Lowering** | node/edge/partition/handler tables, expression trees, `Slot` resolution | `Hir` → `plan::ExecutionPlan` | [identity.md](../spec/identity.md); [execution-model.md](../spec/execution-model.md) | M0 |
| **Guest frontends** | `GuestFrontend` trait — check an opaque guest body against its signature; the host parser never reads it | guest bytes → `plan::GuestProgram` | [stdlib.md §5](../spec/stdlib.md) | guarded |

The facade returns `Result<Compilation, Diagnostics>` with
`Compilation { plan, hash, warnings }`: **a plan with errors is
unrepresentable** — warnings ride the success value, errors preclude
it. Node declarations are an algebraic type,

```text
NodeDecl = Operator(OperatorDecl)
         | Connector(ConnectorDecl { effect : Effect,
                                     behavior : Behavior, … })  — both mandatory
         | Graph(GraphDecl)

Effect   = None | Class(EffectClass)   — `none` is an explicit declared
                                         value, never an absent field;
                                         the three effect *classes* stay three
```

so "an Operator with an effect" or "a Connector without a declaration"
cannot be constructed, and the stdlib's pre-declared `none` connectors
(Clock, Calendar, HttpServer) are representable
([effects-and-sagas.md §2.1–2.2](../spec/effects-and-sagas.md),
[stdlib.md §2](../spec/stdlib.md), [loop.md](../spec/loop.md)). The checker reads Connector
contracts from `stdlib` (§3.7), never from adapter code. `Hir` is the
spec's own name for the checked form ([identity.md §1](../spec/identity.md):
"blake3(canonical HIR)"), and the hash covers the production graph
only — the lab never enters it.

Two compiler rules with consequences elsewhere:

1. **The CEL chain is isolable** (§3.2): the checker hands it a
   *signature*, never an envelope or node.
2. **Desugaring is provable.** Because identity hashes the desugared
   HIR, "sugared and long forms hash identically" is a free, mechanical
   test for every sugar in the table.

### 3.2 The CEL chain (`cel`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`cel`** | the scalar expression language: compile, evaluate, const-evaluate over canonical typed trees | (text, `Signature`) → `Tree`; (`Tree`, `Bindings`) → `Value`; (`Tree`, boot constants) → constant | [grammar.md](../spec/grammar.md) CEL positions; [execution-model.md §5](../spec/execution-model.md) (no ambient time) | M0 subset |

`cel` never sees an envelope, node, or partition, and owns its own
`cel::Diagnostic` (the compiler adapts it into `diagnostics::Diagnostic`).
The *signature* — the typed names visible at one expression position —
is the entire interface, which keeps the chain independently
conformance-testable against the upstream CEL specification. Its value
domain is CEL's (`cel::Type` includes `uint` because CEL has one);
`envelope::Value` is Rastok's data domain — two deliberate domains,
bridged in exactly one place: the `Bindings` adapter the runtime
implements. `cel` is the first candidate for promotion to its own
crate.

### 3.3 The executable plan (`plan`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`plan::ExecutionPlan`** | the executable form — the tables and expression trees the VM interprets, and what ships to other hosts | produced by `compiler`, consumed by `runtime` and `lab` | [identity.md](../spec/identity.md); [execution-model.md](../spec/execution-model.md) | M0 |
| **`plan::Manifest`** | the append-only ledger `SemanticHash → GraphVersions` — plural: a rollback allocates a **new** version for the same hash; a version is never renumbered, never reused | allocated at boot | [identity.md §2](../spec/identity.md) | M0 (minimal) |

`plan::ExecutionPlan` is the whitepaper's term, and the database
intuition it carries is intended — accurate up to two differences:
like a query plan, it is an immutable operator graph — data an engine
interprets, inspects, and ships in fragments to workers; unlike one,
it is the output of a *faithful* lowering (no cost-based rewriting —
the written topology is the executed topology, sealed by the hash)
and it is long-lived: stateful, journaled, versioned, upgraded —
closer to a streaming engine's job graph than to a query's throwaway
plan.

The plan ships unresolved: `env()` and `secret()` stay expression
nodes in the tables, and boot binds them in memory — the same artifact
runs on a laptop and in production.

### 3.4 The runtime machinery (`runtime`)

One vocabulary note: the whitepaper's "the Rastok runtime" names the
*whole* executing system — VM, boundary, connectors, journal; this
module is that system's deterministic machinery, centered on
`runtime::Vm`.

Deterministic by construction: no I/O, no clock reads, no randomness.
Everything nondeterministic enters through the `boundary` traits (§3.5).
The same engine runs `rastok run` and `rastok sim`, through a
debugger-shaped surface: `Vm::boot(&plan, journal)` (construct and
recover, once), `step(event)` (exactly one node transaction), and
`drain()` (run to the stable state — the spec's drain-to-stable). The
lab's Runner is literally a stepper over this surface, which is what
makes single-stepping and journal-relative time travel structural
rather than features.

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **Scheduler** | run-to-completion firing; the single dated-event queue — including the **per-call timeout budgets** (a timeout is the scheduler's dated event, never a partner's answer); virtual-time `advance`; drain-to-stable | plan + ordered events → transaction firings | [execution-model.md §1, §5–6, §8](../spec/execution-model.md) | M0 (single worker) |
| **Mailboxes** | per-key FIFO order, arrival-order capture (the race outcome is journaled), bounded capacity | envelopes → ordered per-key delivery | [execution-model.md §2–3](../spec/execution-model.md) | M0 |
| **TransactionExecutor** | the atomic node transaction: consumed input, state delta, emissions, and records commit together; scratch isolation while a handler runs | firing → transaction commit + emissions | [execution-model.md §1](../spec/execution-model.md) | M0 |
| **EmissionBuffer** | per-port emission identity in text order (`emission_index`) | handler body → ordered emissions | [execution-model.md §4](../spec/execution-model.md) | M0 |
| **Error routing** | typed err values; lexical bubbling to `app.err`; nothing silent | err emissions → routed envelopes | [delivery-semantics.md §5](../spec/delivery-semantics.md) | M0 |
| **`GuestExecutor`** | the trait guest engines implement: run(program, value, budget) — pure, deterministic, budgeted, inside the node transaction | `plan::GuestProgram` → value or typed error | [stdlib.md §5](../spec/stdlib.md) | guarded |
| **`BootBindings`** | `env()` / `secret()` resolution and constant positions, evaluated at boot; a missing or ill-typed binding is a typed refusal, never a default; `Secret<T>` is redacted by type | plan + deployment profile → bound constants | [grammar.md](../spec/grammar.md) initializer positions | next |

### 3.5 The boundary (`boundary`) — the four host interfaces

Every nondeterministic thing crosses exactly one of these four traits,
and what crossed is recorded — the journal's *world-half* is the record
of it. The lab substitutes each one without touching the VM: in run,
the world plays itself; in sim, the lab plays the world.

| Interface | Owns | Real side | Lab side | Spec | M0 |
|---|---|---|---|---|---|
| **`Ingress`** | birth of inbound **drafts** — payload, birth metadata, `IngressAuthority` (opaque), and a sequence request `Next \| Explicit(seq)` (a fixture replays a production slice with explicit sequences); bounded, backpressure translated outward. It never births an envelope: a root envelope **cannot pre-exist its sequence** — the root id derives from `inbound_seq` and `origin_id = id` — so only the journal's accepting transaction mints it | listeners | `lab::Fixture` | [envelope.md §1.1, §2](../spec/envelope.md); [delivery-semantics.md §3, §6](../spec/delivery-semantics.md); [simulation-semantics.md](../spec/simulation-semantics.md) fixtures | M0 (minimal) |
| **`Partner`** | launching journaled `Intent`s and answering with `Response` events carrying the internal `CallRef`; correlation is the **connector's protocol correlator** (HTTP natively, the process boundary in job mode, the declared expression in service mode); a partner deduplicates on the idempotency key only *when it cooperates* | `connectors/*` | `lab::PartnerMock` | [delivery-semantics.md §4](../spec/delivery-semantics.md); [execution-model.md §6.1](../spec/execution-model.md); [stdlib.md §3.3](../spec/stdlib.md) | M0 (Process only) |
| **`Clock`** | the only source of time, delivered **one dated event at a time** — the VM drains to stable between events, which may arm new ones before the target, so a sweep can never be precomputed | wall clock | `lab::VirtualClock` | [execution-model.md §5, §8](../spec/execution-model.md) | M0 (virtual required) |
| **`Supervisor`** | process lifecycle: boot, crash, restart; recovery must never require a clean shutdown | host lifecycle | `lab::FaultInjector` | [delivery-semantics.md §4.4](../spec/delivery-semantics.md); [simulation-semantics.md §5.4](../spec/simulation-semantics.md) | M0 (boot only); faults `next` |

The **`Dispatcher`** (owned here) is the loop between the journal and
the partners: it takes OUTBOUND records the VM opened, drives the
selected `Partner` implementation, and settles every answer or timeout
through `journal::close_outbound(call_ref, …) → Won | AlreadyClosed`.
Only a `Won` closure becomes an input event for the VM; `AlreadyClosed`
degrades the late arrival into a discarded observation — the anti-ghost
rule ([execution-model.md §6.3](../spec/execution-model.md)) as a typed
race, not a convention.

Ownership rules that the types must make unrepresentable:

- a partner **answers or stays silent** — it never returns a timeout
  (the scheduler's dated event decides that) and never returns
  `OutcomeUnknown` (only recovery mints it, §3.6);
- call identity is the internal `CallRef` — a **deterministic
  derivation of the canonical call fields**, never a new serialized
  causal field; the business/idempotency key names the *operation* for
  retries and cooperative partner-side dedup — the two are never
  conflated;
- an inbound envelope is **born inside the accepting transaction**:
  Ingress supplies a draft; only the journal can mint a root id
  (the envelope is immutable and `meta.sys` runtime-owned);
- adapters and mocks produce **events**; the runtime mechanics alone
  write records — a mocked call still gets its OUTBOUND record because
  the VM writes it before any partner, real or scripted, is invoked;
- response and timeout can race, but **exactly one wins** the closure;
  the loser is recorded as a **discarded observation**, never a causal
  fact.

### 3.6 The journal (`journal`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`Journal`** | the typed logical records and their order; `MemJournal` in-memory first | **semantic operations**, not bare appends: `commit(Transaction)` (atomic), `open_outbound(Intent)` (journal-before-effect), `close_outbound(call_ref, response XOR timeout) → Won \| AlreadyClosed` (exactly once — the anti-ghost race is typed), `append_inbound(InboundSubmission) → CommittedInbound { seq, envelope, record }` (in one accepting transaction: allocate or validate the sequence, derive the root id, mint the root `MetaSys` — `parent_id = null`, `origin_id = id` —, build the immutable envelope, commit; only then may the ACK flow), ordered `iter()` | [delivery-semantics.md §1, §4, §6](../spec/delivery-semantics.md); [envelope.md §2, §5](../spec/envelope.md) | M0 |
| **Id derivation** | derived envelope and record ids — zero per-envelope randomness | recipe inputs → `envelope::Id` | [envelope.md §2–3](../spec/envelope.md) | M0 |
| **`RecoveryFold`** | journal → state, dated obligations, open-effect triage — the **only** place `OutcomeUnknown` is minted and an open OUTBOUND becomes `Abandoned`; snapshots are derived accelerations, never causal | records → recovered runtime state | [delivery-semantics.md §4.4, §6–7](../spec/delivery-semantics.md) | next |
| **`GoldenProjection`** | the canonical record projection; the causal-core / annotation split | records → deterministic bytes | [identity.md §3](../spec/identity.md); [simulation-semantics.md §6](../spec/simulation-semantics.md) | M0 |

`InboundRecord` carries the full root recipe — the opaque
`IngressAuthority` (wrapping exactly the normative `(connector_path,
replica)` pair) plus the sequence — so a record autonomously
reconstructs its root id ([identity.md §3](../spec/identity.md)). The
projection lives here because it also serves production replay and
cross-runtime conformance; the golden *workflow* (compare, explicit
bless, diff rendering) is driven by `lab` and `main`.

### 3.7 The stdlib contracts (`stdlib`)

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`ConnectorContract`** | the declarative contract of each stdlib Connector — **all ten mandatory cases**: role, direction, ports, effect, `behavior`, attributes, correlator, error set, instances, journal/simulation | consumed by the checker and by `main`'s wiring; **never depends on adapter code** | [stdlib.md §1–3](../spec/stdlib.md) | M0 (Process only) |
| **`GuestContract`** | the declared guest languages and the **full guest identity the manifest pins**: frontend, executor version, lowering profile, ABI — plus the six-clause purity contract | consumed by the checker and the distribution gate | [stdlib.md §5](../spec/stdlib.md); [identity.md §1](../spec/identity.md) | guarded |

This module answers "how does the compiler know what `Process` is
without linking tokio": the contracts are data, the adapters implement
them, and the checker validates declarations against the spec sheet
alone.

### 3.8 Connectors and guest engines (`connectors`, `guest_engines`)

The effectful and the pure halves of extension, on mirrored seams: a
`connectors/*` adapter implements `boundary::Partner`; a
`guest_engines/*` engine
implements `runtime::GuestExecutor` (and `compiler::GuestFrontend`). A guest
program with I/O belongs behind a Connector, whatever its execution
format.

| Component | Owns | Spec | M0 |
|---|---|---|---|
| **`connectors::Process`** | the first real partner: spawn, observe, typed exit | [stdlib.md §3.3](../spec/stdlib.md) | M0 subset |
| **Catalogue remainder** (HttpServer, HttpClient, Clock, Calendar, Memory) | declared contracts only (see §3.7) | [stdlib.md §3](../spec/stdlib.md) | guarded |
| **`guest_engines::Jq`, `guest_engines::Sql`** | pure guest engines under the six-clause contract | [stdlib.md §5](../spec/stdlib.md) | guarded |

### 3.9 The lab (`lab`)

An adapter set plus a driver — never a second engine.

| Component | Owns | Interface | Spec | M0 |
|---|---|---|---|---|
| **`ScenarioRunner`** | driving the VM through a scenario: inject, advance, expect, golden compare and explicit bless; observation windows; the three expectation quantifiers | scenario → run + verdicts | [simulation-semantics.md §2, §5–6](../spec/simulation-semantics.md) | M0 |
| **Boundary implementations** | `Fixture` (Ingress), `PartnerMock` (Partner — answers only; the VM still writes the OUTBOUND), `VirtualClock` (Clock), `FaultInjector` (Supervisor, `next`) | the scripted external world behind the same four traits | [simulation-semantics.md §2, §5](../spec/simulation-semantics.md) | M0 |
| **Coverage** | the statically enumerable failure-space checklist | plan + scenarios → coverage report | [simulation-semantics.md §7](../spec/simulation-semantics.md) | later |
| **Unsupported surfaces** | `integration`, `theorem`, `experiment`: parse, then refuse explicitly | reachable syntax → `unsupported` diagnostic | [simulation-semantics.md §10–11](../spec/simulation-semantics.md) | guarded |

Scenarios are parsed by the same `compiler` front end as graphs — the
simulation grammar is the graph grammar mirrored — so the lab owns no
second parser and no second engine.

### 3.10 Cross-cutting (`node`, `envelope`, `diagnostics`, `main`)

| Component | Owns | Spec | M0 |
|---|---|---|---|
| **`node`** | the shared vocabulary: `Path` (enters the id recipes), `Kind`, `Port`, `Effect` (`None \| Class(EffectClass)` — `none` explicit, the three classes stay three), `Behavior` (deterministic \| probabilistic — the DSL attribute of the same name), `Trigger` (`p:` \| `l:` — a transaction property, **not** an envelope field) | [grammar.md](../spec/grammar.md) vocabulary; [effects-and-sagas.md §2.2](../spec/effects-and-sagas.md); [stdlib.md §2](../spec/stdlib.md); [envelope.md §3](../spec/envelope.md) | M0 |
| **`envelope`** | `Envelope { meta, data }` where `meta.sys` is the **six-field Tier-1 spine** (`id`, `parent_id`, `origin_id`, `role`, `graph_version`, `mime_type`) plus the Tier-2 birth stamps (`saga_*` / `loop_*`) minted only on envelopes born in an aspect scope ([envelope.md §1.1–1.2](../spec/envelope.md)) and `data` is a two-state `Data` (matching the normative field name): `Encoded { encoding: Encoding (Raw \| Avro \| ArrowIpc), bytes, value_cache }` — original bytes preserved, materialized into `Value` once on first structural access, the cache neither causal nor serialized — **or `Materialized(Value)`** for values born in memory from the graph, encoded only at persistence ([envelope.md §5–5.1](../spec/envelope.md)) | [envelope.md](../spec/envelope.md) | M0 |
| **`diagnostics`** | stable codes, severity, the registry's **phase vocabulary** (`parse`, `resolve`, `check`, `plan`, `run`, `replay`, `sim`), multi-span, structured fields; a check ships with its code, its negative test, and its registry row in the same change | [diagnostics.md](../spec/diagnostics.md) | M0 |
| **`main`** | the `check` / `run` / `sim` verbs, wiring (adapters, dispatcher, stdlib contracts), rendering; owns nothing semantic | — | M0 |

## 4. Dependency direction

```text
main (bin)
   │
   ├──▶ lab (ScenarioRunner + boundary impls) ──┐
   │                                            ├──▶ boundary contracts ──▶ runtime
   └──▶ run wiring (+ stdlib) ──────────────────┘        ▲                     │
                                                         │                     ▼
        connectors ── implement ─────────────────────────┘                 journal
        guest_engines ── implement GuestExecutor ▶ runtime                     ▲
        boundary::Dispatcher ── drives open OUTBOUNDs, closes ─────────────────┘

compiler ──▶ plan (the execution plan) ◀── interprets ──────────────────── runtime
    │    └─▶ stdlib (contract sheets)
    ▼
   cel   (signatures in, trees out — knows nothing else)
```

Rules, in force from the first commit:

1. **`runtime`** depends only on `plan`, the `node`/`envelope` value types,
   the `boundary` contracts it emits into, and the `journal` trait — never
   on an adapter, the lab, or the compiler.
2. **`cel`** depends on nothing else in Rastok.
3. **Adapters** (`connectors`, `guest_engines`, the lab's boundary
   implementations) implement traits; nothing depends on an adapter
   except `main`'s wiring. The `Dispatcher` is the only `boundary`
   component that touches `journal` — through the `Journal` trait,
   never a concrete store.
4. **`stdlib`** is declarative data over the base vocabulary; the
   checker and `main` read it; it depends on no adapter.
5. **`lab`** depends on `compiler`, the `boundary` contracts, and
   `journal`. It never patches or introspects `runtime` internals.
6. **`node`, `envelope`, `diagnostics`** are the base layer: depended on
   everywhere, depending on nothing (except `envelope` on `node`).
7. **`journal::GoldenProjection`** reads the journal, not the VM.

Within one crate these rules are held by review, not by Cargo — module
cycles are legal in Rust. Promotion to crates (per the crate policy)
is what will eventually make them mechanical.

## 5. Seams that keep later milestones possible

M0 builds none of the following capabilities, but its APIs must not
foreclose them. These are obligations on M0 code, not features.

- **Logical journal, physical later.** `journal::Journal` exposes
  semantic operations over typed records. Segment files,
  snapshots-on-disk, and retention arrive later behind the same trait
  (`ARCHITECTURE.md`, boundary rule 5).
- **Opaque origin authorities.** `boundary::IngressAuthority` wraps
  exactly the normative `(connector_path, replica)` pair
  ([identity.md §3](../spec/identity.md)) but stays opaque: M0 code
  never assumes the replica component is a constant it may inline.
  This keeps the multi-host door open without building anything behind
  it.
- **One canonical encoder.** Every canonical form — HIR, plan, record
  projection — uses the same deterministic encoding discipline with
  per-form, extend-only key tables
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
| Parse | `compiler` (parse), `diagnostics` |
| Check | `compiler` (assemble, desugar, check vs `stdlib` contracts), `cel` (compile) |
| Plan | `compiler` (canonicalize, lower), `plan::Manifest` |
| Step | `runtime` (Scheduler, Mailboxes, TransactionExecutor, EmissionBuffer, error routing) |
| Handle | `boundary::Dispatcher` + `boundary::Partner`; `connectors::Process` (App) or `lab::PartnerMock` (Lab) |
| Record | `journal` (semantic operations, id derivation) |
| Compare | `journal::GoldenProjection` projection, `lab`/`main` workflow |

Deliberately without M0 code: `journal::RecoveryFold` and `lab::FaultInjector`
(`next` — the record API already carries their inputs), `runtime::BootBindings`
(`next`), `guest_engines` and the connector catalogue beyond Process
(`guarded`), coverage (`later`), and live Saga and Loop managers
(deferred per [`spec-to-code.md`](spec-to-code.md); the checker still
validates or explicitly refuses their syntax within the M0 subset).
