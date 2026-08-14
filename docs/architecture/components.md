# Component blueprint

> Implementation blueprint — non-normative. Originally written ahead
> of the build; updated through the landed M1.3 B1 value completion and B3
> expression core on 2026-08-14. The M0 deviations remain recorded in
> [`m0-exit-notes.md`](m0-exit-notes.md), while the chronological M1
> evidence lives in [`m1-slice.md`](m1-slice.md). The specified M1.3
> replacement of the type/value and boundary seams is tracked in
> [`m1.3-type-system-backlog.md`](m1.3-type-system-backlog.md); this file
> describes the current v6 implementation until that landing is complete.

This document refines the responsibility table of
[`ARCHITECTURE.md`](../../ARCHITECTURE.md) to the component level: the
data forms that flow through the system, the Rust modules and types
that own them, the dependency direction between modules, and what the
current build implements. It adds no language or runtime semantics;
on any conflict,
[`docs/spec/`](../spec/README.md) wins. The
[C4 component view](03-rastok-components.svg) remains the coarse visual.
The [pre-build module sketch](05-rastok-packages.svg) is archived for
design history; its names are not a current API inventory. The maintained
[module overview](06-rastok-modules-overview.svg) tracks the load-bearing
modules and public types of the current build.

## How to read this

- A **component** is a responsibility with an owned data form and a
  typed interface. Components live as modules — or private types —
  in the `rastok-core` kernel or the `rastok` std/compiler layer; the
  crate policy of `ARCHITECTURE.md` governs any further promotion.
- **Interfaces are data.** Modules hand each other the immutable forms
  of §1. No module reaches into another's internals through callbacks
  or shared mutable state.
- **Status legend** — `M0`: built by the first slice, possibly as a
  reduced subset; `M1.1` / `M1.2`: landed by that M1 item;
  `next`: the next named slice; `guarded`: no implementation, yet
  reachable syntax produces an explicit diagnostic, never an accident;
  `later`: absent, no stub.

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
| `plan::ValidatedExecutionPlan` | `compiler` (lower, then validate) or `plan::decode` | `runtime`, `lab`, `plan::encode` | the only executable/package boundary: an immutable `ExecutionPlan` whose tables, `expr::Tree`s, slots, contracts, and structural budgets have already passed the plan gate |
| `boundary::Intent` (open OUTBOUND) | `runtime` | dispatcher → `boundary::Partner` impls | journaled before any handler runs, carrying the canonical call fields — emitting id, connector path, request digest, optional idempotency key ([delivery-semantics.md §4](../spec/delivery-semantics.md); [identity.md §3](../spec/identity.md)) |
| `boundary::Response` | `connectors` or the lab's mock partner | `runtime` (as an input event) | returns with the internal `CallRef`, correlated by the *connector's own protocol correlator* — never by the business key; a partner only ever answers: it never decides a timeout and never mints `OutcomeUnknown` |
| `journal::Record` | **runtime mechanics only** | recovery, `golden::Trace` | adapters and mocks supply events, never records; causal core and observation annotations kept separable ([identity.md §3](../spec/identity.md)) |
| `diagnostics::Diagnostic` | every phase | entry points | stable code, spans, structured fields; accumulated values — never prints, never panics |
| golden projection | `golden::Trace` (project, seal, diff) | evidence compare | deterministic bytes: same scope, same bytes ([simulation-semantics.md §6](../spec/simulation-semantics.md)) |

## 2. The module map

```text
crates/rastok-core/src/       # the kernel — #![no_std] + alloc (split at M0.5)
├── datatype.rs, value.rs       # logical types, values, validation/conformance
├── node.rs, envelope.rs, expr.rs, plan.rs
├── canonical_cbor.rs         # one bounded canonical byte discipline
├── identity/encoding.rs      # tagged-value identity registry
├── json.rs, trivia.rs        # shared exact JSON and lexical-comment facts
├── journal.rs                # records (+ armed timers), trait, ids, MemJournal
├── boundary.rs               # host vocabulary, request framing, five traits
└── runtime.rs, runtime/vm.rs # the machine

crates/rastok/src/            # the std layer — re-exports the kernel
├── diagnostics.rs    # Diagnostic, Code, Severity, Phase, Span, Bag
├── compiler.rs       # facade : parse → check → hash → lower → validate
│   └── compiler/     #   lexer (cursor + lookahead), parser (lexemes only),
│                     #   syntax, hir (graph + lab), check, lower
├── contract.rs       # the node-contract METAMODEL (D-2026-08-12-1): AttributeSpec,
│                     #   Presence, DefaultValue, ValueDomain, profiles, Registry,
│                     #   validate_sheet — compiler-layer data, never serialized
├── stdlib.rs         # the standard CATALOGUE instancing that metamodel (PROCESS,
│                     #   the declared names, GuestContract, Registry::try_m0())
├── golden.rs         # Trace, Divergence — projection, seal, named diff
├── connectors.rs     # Process: bind effective settings, then async transport
├── conformance.rs    # adapter contract harness over the production seam
├── lab.rs            # run → ScenarioRun { report, records } ; the mock partner
├── render.rs         # Sources + rustc-shaped diagnostic rendering
└── main.rs           # thin bin : check | run | simulate (--golden / --bless)
```

Two deviations from the pre-build sketch, both deliberate: the golden
projection grew into its own `golden` module (it owns projection and
named diff while sharing the kernel's two-way canonical CBOR codec),
and `guest_engines.rs` was never created — a
guarded surface needs a diagnostic, not an empty file.

Naming rules — **M0 working conventions**, retained after the first
crate promotion (the final layout stays open per `ARCHITECTURE.md`):

1. **Spec-first names.** A module carries the name of the concept that
   owns its invariant in the normative corpus (`envelope`, `journal`,
   `lab`, `node`, `stdlib`) or in the whitepaper (`plan` — the
   executable plan); purely internal machinery takes plain engineering
   names (`compiler`, `runtime`, `boundary`, `expr`).
2. **Explicit English over abbreviations.** A newcomer must understand
   a name without the project's internal vocabulary: `diagnostics`,
   `guest_engines`, `ScenarioReport`, `RecoveryFold`. No blessed
   abbreviations remain.
3. **Qualified paths give context; precision beats brevity.** A type
   never repeats its module name (`expr::Tree`, never `ExpressionTree`), but
   keeps a precise compound when the bare word would be too generic
   (`lab::ScenarioRun`, `golden::Divergence`) and keeps the
   corpus term even when it overlaps the module word
   (`plan::ExecutionPlan`, `plan::SemanticHash`). The `x::X` stutter
   stays idiomatic where the type *is* the module's whole concept:
   `journal::Journal`, `envelope::Envelope`.
4. **Flat, depth ≤ 2.** A directory exists for exactly two reasons — a
   catalogue (one file per item: `connectors/`, `guest_engines/`) or an
   overflow split behind an unchanged facade (`compiler/`). Never
   created in advance.
5. **One engine.** There is no simulator component: `rastok simulate`
   drives the same `runtime`, injects/advances through its public VM
   surface, and substitutes the `Partner` with the lab's mock.

## 3. Component inventory

Columns: what the component owns, its interface stated as data in →
data out, the governing spec, and current status.

### 3.1 The compiler (`compiler`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **Lexer + parser** | one forward byte cursor and token lookahead in `Lexer`; complete host lexemes; specialized expression/JSON/verbatim/guest regions; spans and structural recovery | source bytes → syntax tree + diagnostics | [grammar.md](../spec/grammar.md); D-2026-08-13-1 | M1.2 interlude, naming aligned in B3 |
| **Unit assembly** | the deployed unit: multi-file resolution, lab-fragment merge, exactly-one-App, wiring by name, no shadowing | syntax trees → one resolved `Unit` | [grammar.md](../spec/grammar.md) scope rules; [simulation-semantics.md §2](../spec/simulation-semantics.md) | M0 (single unit) |
| **Desugarer** | the finite sugar table — afterwards there is one way to say each thing | resolved unit → core-form graph | [grammar.md](../spec/grammar.md) | M0 |
| **Type checker** | envelope/data types, `sys`/`app`/`ext` metadata grid, named structural types, contract attributes/defaults/profiles, mandatory legacy err sets, typed `is`/`as` narrowing, aspect admission | core-form graph → `Hir` + diagnostics | [grammar.md](../spec/grammar.md); [expressions.md](../spec/expressions.md); [aspects.md](../spec/aspects.md); [effects-and-sagas.md](../spec/effects-and-sagas.md) | M1.3 B3; new trees gated from v6 until B8 |
| **Canonicalizer** | the canonical byte form of the resolved HIR; the semantic hash | `Hir` → canonical bytes + `plan::SemanticHash` | [identity.md §1](../spec/identity.md) | M0, extended in M1 |
| **Lowering + plan gate** | node/edge/handler tables, expression trees, `Slot` resolution, schemas and assets; structural validation before success | `Hir` → `plan::ValidatedExecutionPlan` | [identity.md](../spec/identity.md); [execution-model.md](../spec/execution-model.md); [plan-package.md](plan-package.md) | M0.5, extended in M1 |
| **Guest frontends** | future registered frontend: check one opaque guest-region lexeme against its signature; the host never tokenizes its body | guest bytes → future compiled guest program | [stdlib.md §5](../spec/stdlib.md) | guarded; no placeholder trait |

The facade returns `Result<Compilation, Diagnostics>` with
`Compilation { plan, symbols, hash, warnings, labs }`: **a plan with errors is
unrepresentable** — warnings ride the success value, errors preclude
it, and `plan` already has type `ValidatedExecutionPlan`. Node declarations are an algebraic type,

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
cannot be constructed. `Effect::None` also represents the catalogue's
pure-source connectors (Clock, Calendar, HttpServer) without turning
`none` into a fourth effect class; their current declared-only status is
still rejected before HIR construction
([effects-and-sagas.md §2.1–2.2](../spec/effects-and-sagas.md),
[stdlib.md §2](../spec/stdlib.md), [loop.md](../spec/loop.md)). The checker reads Connector
contracts from `stdlib` (§3.7) through the `contract` metamodel,
never from adapter code — and it INJECTS every omitted `Defaulted`
attribute into the effective HIR (D-2026-08-12-1), so the plan
carries effective attributes only and adapters never reintroduce a
default. `Hir` is the
spec's own name for the checked form ([identity.md §1](../spec/identity.md):
"blake3(canonical HIR)"), and the hash covers the production graph
only — the lab never enters it.

Two compiler rules with consequences elsewhere:

1. **The current expression chain is isolable** (§3.2): the checker hands it a
   *signature*, never an envelope or node.
2. **Desugaring is provable.** Because identity hashes the desugared
   HIR, "sugared and long forms hash identically" is a free, mechanical
   test for every sugar in the table.

### 3.2 The expression chain (`expr`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`expr`** | the expression compiler/evaluator over core `DataType` and `Value`; its evaluator-only carrier may additionally hold the lazy payload witness | (text, `Signature`) → `Tree`; (`Tree`, `Bindings`) → evaluation result; (`Tree`, boot constants) → core value | [expressions.md](../spec/expressions.md); [execution-model.md §5](../spec/execution-model.md) (no ambient time) | M1.3 B3 core landed; v7 transport remains B8 |

`expr` never sees an envelope, node, or partition, and owns its own
`expr::Diagnostic` (the compiler adapts it into `diagnostics::Diagnostic`).
The *signature* — the typed names visible at one expression position —
is the entire interface, which keeps the chain independently
conformance-testable against the compatible upstream CEL subset. Its
signatures and trees consume core `datatype::DataType` and `value::Value`; it
owns neither semantic domain. `envelope::Value` is a re-export of that one
core value type. `expr::EvalValue` exists only inside evaluation because the
lazy whole-payload witness may flow through locals and expressions; conversion
to a core value rejects any unresolved witness. Inside the kernel the chain
also shares foundational utilities: canonical nesting/double rules, the exact
number reader, and the single comment recognizer.

Core also owns the complete strict-profile value judges: `Value::validate`
rejects malformed materialized values independently of where they entered,
while `DataType::violation` reports recursive conformance failures — including
float widths, temporal shapes, UUID and canonical-key maps — with their nested
path. The plan gate, metadata/state doors, VM ingress and emission gate consume
those same operations; JSON retains only its origin-specific number and
document rules.

The boundary rename and typed `is`/`as` narrowing are landed. The compatible
operator corpus is executed against a pinned independent CEL oracle; Rastok's
extensions and divergences are listed in `expressions.md`. v6 validation
categorically refuses the new AST forms, so only B8 assigns their package ids.

### 3.3 The executable plan (`plan`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`plan::ValidatedExecutionPlan`** | the executable/package form — a private raw `ExecutionPlan` plus a total effective `ResolvedPortTable`; construction validates structure, solves every used/declared port, and rejects unresolved or conflicting constraints | produced by `compiler` or `plan::decode`; consumed by `runtime`, `lab`, and `plan::encode` | [types.md](../spec/types.md); [identity.md](../spec/identity.md); [plan-package.md](plan-package.md) | M1.3 B2; v6 re-derives the table, v7 will transport it |
| **`plan::Manifest`** | the append-only ledger `SemanticHash → GraphVersions` — plural: a rollback allocates a **new** version for the same hash; a version is never renumbered, never reused | allocated at boot | [identity.md §2](../spec/identity.md) | M0 (minimal) |

`ExecutionPlan` remains the whitepaper's domain term and the private data
validated by the gate; `ValidatedExecutionPlan` is the only type allowed
to cross into execution or package encoding. The database intuition is
intended — accurate up to two differences:
like a query plan, it is an immutable operator graph — data an engine
interprets, inspects, and ships in fragments to workers; unlike one,
it is the output of a *faithful* lowering (no cost-based rewriting —
the written topology is the executed topology, sealed by the hash)
and it is long-lived: stateful, journaled, versioned, upgraded —
closer to a streaming engine's job graph than to a query's throwaway
plan.

The current build does **not** ship unresolved `env()` or `secret()`
nodes: those calls are guarded and refuse before a plan exists.
`BootBindings` remains the intended future boundary; when that slice
lands, deployment values must bind outside semantic identity so one
package can run under different profiles without hiding an ambient
environment read in the core.

### 3.4 The runtime machinery (`runtime`)

One vocabulary note: the whitepaper's "the Rastok runtime" names the
*whole* executing system — VM, boundary, connectors, journal; this
module is that system's deterministic machinery, centered on
`runtime::Vm`.

Deterministic by construction: no I/O, no clock reads, no randomness.
Everything nondeterministic enters through the `boundary` traits (§3.5).
The same engine runs `rastok run` and `rastok simulate`, through a
debugger-shaped surface: `Vm::boot(&validated_plan, graph_version)`
constructs the machine; `inject`, `step`, `drain`, `pump`, and `advance`
receive the journal explicitly at each causal operation. Recovery is
not hidden in construction: `RecoveryFold` remains a later slice and
must restore the machine through an explicit journal-facing boundary.
The lab's Runner is literally a stepper over this surface, which keeps
single-stepping and future journal-relative inspection structural.

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **Scheduler** | run-to-completion firing; the single dated-event queue — the **call budgets, armed at DELIVERY** so queue time counts, referenced by `(edge, envelope id)`, resolved reserve→resolve→ack so a refused expiry replays (a timeout is the scheduler's dated event, never a partner's answer); virtual-time `advance`; drain-to-stable | plan + ordered events → transaction firings | [execution-model.md §1, §5–6, §8](../spec/execution-model.md) | M0 (single worker) |
| **Mailboxes** | per-key FIFO order, arrival-order capture (the race outcome is journaled), bounded capacity | envelopes → ordered per-key delivery | [execution-model.md §2–3](../spec/execution-model.md) | M0 |
| **TransactionExecutor** | the atomic node transaction: consumed input, state delta, emissions, and records commit together; scratch isolation while a handler runs | firing → transaction commit + emissions | [execution-model.md §1](../spec/execution-model.md) | M0 |
| **EmissionBuffer** | per-port emission identity in text order (`emission_index`) | handler body → ordered emissions | [execution-model.md §4](../spec/execution-model.md) | M0 |
| **Legacy failure routing** | connector- and machine-produced failures currently take a dedicated success/failure seam; M1.3 replaces it with ordinary typed port outputs and a separate completion control fact | current typed outcomes → routed envelopes | [types.md](../spec/types.md); [m1.3 backlog](m1.3-type-system-backlog.md) | current scaffold; removed by M1.3 |
| **`GuestExecutor`** | the existing runtime seam future guest engines implement: `run(&[u8], value, budget)` — pure, deterministic, budgeted, inside the node transaction; the bytes become a typed/versioned `GuestProgram` only when that slice lands | opaque program bytes → value or typed error | [stdlib.md §5](../spec/stdlib.md) | seam only; no engine/program type yet |
| **`BootBindings`** | future `env()` / `secret()` resolution at boot; a missing or ill-typed binding will be a typed refusal, never a default; `Secret<T>` remains a design requirement, not a current type | plan + deployment profile → bound constants | [grammar.md](../spec/grammar.md) initializer positions | guarded / later |

### 3.5 The boundary (`boundary`) — the four host interfaces

Every nondeterministic thing is represented at one of these host
boundaries, and what crosses into causality is recorded — the journal's
*world-half* is the record of it. The current lab substitutes the
`Partner`, injects ingress drafts through `Vm::inject`, and drives virtual
time through `Vm::advance`; the declared `Clock` and `Supervisor` traits
remain host seams rather than objects the M1 VM consumes directly.

| Interface | Owns | Real side | Lab side | Spec | Status |
|---|---|---|---|---|---|
| **`Ingress`** | birth of inbound **drafts** — payload, birth metadata, `IngressAuthority` (opaque), and a sequence request `Next \| Explicit(seq)` (a fixture can replay a production slice with explicit sequences); bounded, backpressure translated outward. It never births an envelope: a root envelope **cannot pre-exist its sequence** — the root id derives from `inbound_seq` and `origin_id = id` — so only the journal's accepting transaction mints it | future listener adapters | scenario injection through `Vm::inject` (no `Fixture` type) | [envelope.md §1.1, §2](../spec/envelope.md); [delivery-semantics.md §3, §6](../spec/delivery-semantics.md); [simulation-semantics.md](../spec/simulation-semantics.md) fixtures | boundary vocabulary + VM entry; no ingress adapter yet |
| **`Partner`** | launching journaled `Intent`s and answering with `Response` events carrying the internal `CallRef`; correlation is the **connector's protocol correlator** (HTTP natively, the process boundary in job mode, the declared expression in service mode); a partner deduplicates on the idempotency key only *when it cooperates*; `cancel(call_ref)` withdraws in-flight work when the machine closes a call (default: nothing) | `connectors::Process` | the lab's mock partner | [delivery-semantics.md §4](../spec/delivery-semantics.md); [execution-model.md §6.1](../spec/execution-model.md); [stdlib.md §3.3](../spec/stdlib.md) | M0 (Process only) |
| **`Clock`** | the only source of time. As built, M0 consumes no trait impl: `Vm::advance(journal, partner, target)` IS the virtual clock — the driver (a lab step, the CLI's wall-mapping loop) chooses targets, due budgets fire in expiry order, and same-instant expiries all fire before the machine drains | declared trait; `advance` targets in M0 | `advance` calls | [execution-model.md §5, §8](../spec/execution-model.md) | M0 (target-driven) |
| **`Supervisor`** | process lifecycle: boot, crash, restart; recovery must never require a clean shutdown | host lifecycle | not built; fault steps remain guarded | [delivery-semantics.md §4.4](../spec/delivery-semantics.md); [simulation-semantics.md §5.4](../spec/simulation-semantics.md) | declared seam; current wiring boots only |

The **`Dispatcher`** (owned here) is the loop between the journal and
the partners: it takes OUTBOUND records the VM opened, drives the
selected `Partner` implementation, and settles answers through
`journal::close_outbound(call_ref, …) → Won | AlreadyClosed`. Before
any closure, the **`OutcomeGate`** judges the reply against the
calling connector's sealed error set — a refused reply never becomes
a causal fact: it returns in `PumpBatch::rejected` with its reason,
and the call stays open, honestly settleable by the timeout. Only a
`Won` closure becomes an input event for the VM; `AlreadyClosed`
degrades the late arrival into a discarded observation — the
anti-ghost rule
([execution-model.md §6.3](../spec/execution-model.md)) as a typed
race, not a convention. `PumpBatch` returns every non-causal
observation — discarded, uncorrelated, rejected — nothing silent.

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

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`Journal`** | the typed logical records and their order; `MemJournal` in-memory first | **semantic operations**, not bare appends: `commit(Transaction)` (atomic), `open_outbound(Intent)` (journal-before-effect), `close_outbound(call_ref, response XOR timeout) → Won \| AlreadyClosed` (exactly once — the anti-ghost race is typed), `append_inbound(InboundSubmission) → CommittedInbound { seq, envelope, record }` (in one accepting transaction: allocate or validate the sequence, derive the root id, mint the root `MetaSys` — `parent_id = null`, `origin_id = id` —, build the immutable envelope, commit; only then may the ACK flow), ordered `iter()` | [delivery-semantics.md §1, §4, §6](../spec/delivery-semantics.md); [envelope.md §2, §5](../spec/envelope.md) | M0 |
| **Id derivation** | derived envelope and record ids — zero per-envelope randomness | recipe inputs → `envelope::Id` | [envelope.md §2–3](../spec/envelope.md) | M0 |
| **`RecoveryFold`** | journal → state, dated obligations, open-effect triage — the **only** place `OutcomeUnknown` is minted and an open OUTBOUND becomes `Abandoned`; snapshots are derived accelerations, never causal | records → recovered runtime state | [delivery-semantics.md §4.4, §6–7](../spec/delivery-semantics.md) | next |
| *(golden projection)* | moved to its own `golden` module as built — see §3.6b | | | |

### 3.6b The golden evidence (`golden`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`golden::Trace`** | the canonical record projection (identity.md §3 tables, armed timers included) and blake3 seal, encoded/decoded through the kernel's shared canonical CBOR | records → deterministic bytes; stored golden → structural re-read | [identity.md §3](../spec/identity.md); [simulation-semantics.md §6](../spec/simulation-semantics.md) | M0.5 |
| **`golden::Divergence`** | the first meaningful difference, by record index and NAMED key path — a broken seal explains itself, never a hex dump | two traces → divergence or none | — | M0 |

The module owns both directions of the **trace projection** and uses the
shared canonical CBOR reader/writer, so a mismatch diffs structurally
against the STORED golden; the workflow (compare, explicit `--bless`)
is driven by the lab's returned records, the golden tests, and `main`.

`InboundRecord` carries the full root recipe — the opaque
`IngressAuthority` (wrapping exactly the normative `(connector_path,
replica)` pair) plus the sequence — so a record autonomously
reconstructs its root id ([identity.md §3](../spec/identity.md)). The
projection lives here because it also serves production replay and
cross-runtime conformance; the golden *workflow* (compare, explicit
bless, diff rendering) is driven by `lab` and `main`.

### 3.7 The stdlib contracts (`stdlib`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`contract::ConnectorContract`** | the compiler-facing contract sheet: ports, common attributes, optional profile selector, effective presence/default/domain rules, and typed error set. The ten-case prose template remains the normative design review checklist; only executable checker facts belong in this type | consumed by the checker and adapter binding; **never depends on adapter code** | [stdlib.md §1–3](../spec/stdlib.md); D-2026-08-12-1 | M1.2 interlude (`Process` sheet only) |
| **`stdlib::GuestContract`** | the currently declared guest name and availability bit. Version/ABI/lowering identity joins this model only with a real frontend and executor | consumed by the checker | [stdlib.md §5](../spec/stdlib.md); [identity.md §1](../spec/identity.md) | guarded |

This module answers "how does the compiler know what `Process` is
without linking tokio": the contracts are data, the adapters implement
them, and the checker validates declarations against the spec sheet
alone.

### 3.8 Connectors and guest engines (`connectors`, `guest_engines`)

The effectful and the pure halves of extension, on mirrored seams: a
`connectors/*` adapter implements `boundary::Partner`; a future
`guest_engines/*` engine implements `runtime::GuestExecutor` and will
pair with a compiler frontend registered only when the first real guest
implementation lands. No placeholder `GuestFrontend` type exists in the
current compiler. A guest program with I/O belongs behind a Connector,
whatever its execution format.

| Component | Owns | Spec | Status |
|---|---|---|---|
| **`connectors::Process`** | the real job-mode partner: pre-binds every effective setting before the VM/journal exists, spawns asynchronously, captures under one aggregate budget, and supports `lines` / `jsonl` | [stdlib.md §3.3](../spec/stdlib.md) | M1.2 |
| **Catalogue remainder** (HttpServer, HttpClient, Clock, Calendar, Memory, File) | names declared so source refuses explicitly; contract sheets and adapters land by slice | [stdlib.md §3](../spec/stdlib.md) | guarded |
| **Future jq/sql engines** | pure guest engines under the six-clause contract; no `guest_engines` module exists yet | [stdlib.md §5](../spec/stdlib.md) | guarded |

### 3.9 The lab (`lab`)

An adapter set plus a driver — never a second engine.

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`lab::run`** | driving the production `Vm` through each checked scenario: inject, advance, settle, expectation windows over the machine's observation log, the three quantifiers; every uncovered call, faulting predicate or guard, and unsettled machine is a named failure | `hir::Lab` + plan → `Vec<ScenarioRun { report, records }>` — the records feed the golden workflow | [simulation-semantics.md §2, §5–6](../spec/simulation-semantics.md) | M0 |
| **The scripted world** | as built, leaner than sketched: the *fixture* IS the scenario's steps (injections through `Vm::inject` — no `Fixture` type); the *virtual clock* IS `Vm::advance` (no `VirtualClock` type); the internal mock partner answers behind the same `Partner` seam as the real adapter — replies only, the VM still writes every OUTBOUND; the fault injector stays `next` | the scripted external world | [simulation-semantics.md §2, §5](../spec/simulation-semantics.md) | M0 |
| **Coverage** | the statically enumerable failure-space checklist | plan + scenarios → coverage report | [simulation-semantics.md §7](../spec/simulation-semantics.md) | later |
| **Unsupported surfaces** | `integration`, `theorem`, `experiment`: parse, then refuse explicitly | reachable syntax → `unsupported` diagnostic | [simulation-semantics.md §10–11](../spec/simulation-semantics.md) | guarded |

Scenarios are parsed by the same `compiler` front end as graphs — the
simulation grammar is the graph grammar mirrored — so the lab owns no
second parser and no second engine.

### 3.10 Cross-cutting (`node`, `envelope`, `diagnostics`, `main`)

| Component | Owns | Spec | Status |
|---|---|---|---|
| **`node`** | the shared vocabulary: `Path` (enters the id recipes), `Kind`, `Port`, `Effect` (`None \| Class(EffectClass)` — `none` explicit, the three classes stay three), `Behavior` (deterministic \| probabilistic — the DSL attribute of the same name), `Trigger` (`p:` \| `l:` — a transaction property, **not** an envelope field) | [grammar.md](../spec/grammar.md) vocabulary; [effects-and-sagas.md §2.2](../spec/effects-and-sagas.md); [stdlib.md §2](../spec/stdlib.md); [envelope.md §3](../spec/envelope.md) | M0 |
| **`envelope`** | `Envelope { meta, data }` where `meta.sys` is the **five-field Tier-1 spine** (`id`, `parent_id`, `origin_id`, `role`, `graph_version` — the content type lives in `meta.ext.content_type`, D-2026-08-10-4) plus the Tier-2 birth stamps (`saga_*` / `loop_*`) minted only on envelopes born in an aspect scope ([envelope.md §1.1–1.2](../spec/envelope.md)) and `data` is a two-state `Data` (matching the normative field name): `Encoded { encoding: Encoding (Raw \| Avro \| ArrowIpc), bytes, value_cache }` — original bytes preserved, materialized into `Value` once on first structural access, the cache neither causal nor serialized — **or `Materialized(Value)`** for values born in memory from the graph, encoded only at persistence ([envelope.md §5–5.1](../spec/envelope.md)) | [envelope.md](../spec/envelope.md) | M0 |
| **`diagnostics`** | stable codes, severity, the registry's **phase vocabulary** (`parse`, `resolve`, `check`, `plan`, `run`, `replay`, `sim`), multi-span, structured fields; a check ships with its code, its negative test, and its registry row in the same change | [diagnostics.md](../spec/diagnostics.md) | M0 |
| **`main`** | the `check` / `run` / `simulate` verbs, wiring (adapters, dispatcher, stdlib contracts), rendering; owns nothing semantic | — | M0 |

## 4. Dependency direction

```text
main (bin)
   │
   ├──▶ lab (run + the mock partner) ───────────┐
   │                                            ├──▶ boundary contracts ──▶ runtime
   └──▶ run wiring (+ stdlib) ──────────────────┘        ▲                     │
                                                         │                     ▼
        connectors ── implement ─────────────────────────┘                 journal
        future guest engines ── implement GuestExecutor ▶ runtime               ▲
        boundary::Dispatcher ── drives open OUTBOUNDs, closes ─────────────────┘

compiler ──▶ plan (the execution plan) ◀── interprets ──────────────────── runtime
    │    └─▶ stdlib (contract sheets)
    ▼
   expr  (signatures in, trees out; shares only canonical/json/trivia foundations)
```

Rules, in force from the first commit:

1. **`runtime`** depends only on `plan`, the `node`/`envelope` value types,
   the `boundary` contracts it emits into, and the `journal` trait — never
   on an adapter, the lab, or the compiler. The M0 debt is closed:
   `boundary::prepare_request` creates one `PreparedRequest`, so the
   digest and transported bytes are the same sealed value without a
   VM→`connectors` dependency.
2. **`expr`** depends on no graph/runtime concept; it shares only
   `canonical_cbor`, `json`, and `trivia` primitives from the kernel.
3. **Adapters** (`connectors`, `guest_engines`, the lab's boundary
   implementations) implement traits; nothing depends on an adapter
   except `main`'s wiring. The `Dispatcher` is the only `boundary`
   component that touches `journal` — through the `Journal` trait,
   never a concrete store.
4. **`stdlib`** is declarative data over the base vocabulary; the
   checker and `main` read it; it depends on no adapter.
5. **`lab`** depends on `compiler`, the `boundary` contracts, and
   `journal`. It never patches or introspects `runtime` internals.
6. **`node`, `envelope`, `diagnostics`** are shared foundations for the
   layers that need them; `node` is leaf-like, `envelope` builds on it,
   and diagnostics stays in the compiler/std crate rather than entering
   the `no_std` runtime vocabulary.
7. **`golden`** reads journal records, not the VM.

The `rastok-core` / `rastok` split now makes the most important frontier
mechanical: the `no_std` kernel cannot import the compiler, adapters, lab,
or OS layer. Dependencies within either crate still require review because
Rust modules may refer across their tree.

## 5. Seams that keep later milestones possible

This section preserves the seams identified at M0 exit. Some are now
realized (the core split and shared canonical codec); the remaining
items still constrain later slices.

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
  reachable-but-unbuilt surface produces its explicit diagnostic;
  `env()` / `secret()` currently refuse before a plan exists until the
  boot-binding slice lands. Silence is the only forbidden outcome.

## 6. M0 coverage map

The slice stages of [`m0-slice.md`](m0-slice.md), in module terms:

| Slice stage | Exercised |
|---|---|
| Parse | `compiler` (parse), `diagnostics` |
| Check | `compiler` (assemble, desugar, check vs `stdlib` contracts), `expr` (compile) |
| Plan | `compiler` (canonicalize, lower), `plan::Manifest` |
| Step | `runtime` (Scheduler, Mailboxes, TransactionExecutor, EmissionBuffer, error routing) |
| Handle | `boundary::Dispatcher` + `boundary::Partner` behind the OutcomeGate; `connectors::Process` (App) or the lab mock (Lab) |
| Record | `journal` (semantic operations, id derivation) |
| Compare | `golden` projection, seal, and diff; `lab`-returned records; `main --golden/--bless` |

Deliberately without M0 code: `journal::RecoveryFold` and the lab's fault injector
(`next` — the record API already carries their inputs), `runtime::BootBindings`
(`next`), `guest_engines` and the connector catalogue beyond Process
(`guarded`), coverage (`later`), and live Saga and Loop managers
(deferred per [`spec-to-code.md`](spec-to-code.md); the checker still
validates or explicitly refuses their syntax within the M0 subset).

## 7. M1 state at this revision

The chronological detail and commit evidence remain in
[`m1-slice.md`](m1-slice.md). The architectural delta is shorter:

- **M1.1:** named boundary ingress and typed/pinned multiport tables
  travel in the validated plan; every emission crosses one conformance
  gate.
- **M1.2:** `sys`/`app`/`ext` metadata, exact JSON/decimal values,
  assets, named structural types, and the Process `jsonl` codec are in
  the executable subset. The package is format v6.
- **Compiler interludes:** `contract` is the sole node-attribute
  metamodel; defaults are injected before identity, and the parser is a
  forward lexeme-stream parser. The lexer owns cursor/lookahead and
  delegates expression, JSON, verbatim, and guest regions without rewind.
- **M1.3 foundation:** one complete strict-profile `DataType`/`Value`
  ownership, validation and recursive conformance authority serves plans,
  state, metadata, expressions and VM doors. B2 adds a total effective port table,
  with declared/inferred origin, and makes unresolved/conflicting constraints
  typed refusals without a package-format move. B3 renames the consumer to
  `expr`, adds deterministic typed narrowing, and keeps its new AST behind the
  v7 package gate.
- **Next:** M1.3 lands tagged frontends, uniform connector port contracts,
  external JSON/Avro/Arrow schema adapters, multi-output boundary completion
  and v7.
  `per`, joins, new adapters,
  Loop, counters, and `--watch` remain guarded in the order recorded by
  the slice.
