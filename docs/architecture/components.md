# Component blueprint

> Current implementation blueprint — non-normative, updated 2026-10-07.
> The [format registry](../spec/identity.md) is the authority for current
> package and record shapes. Process job/service use `std/process@4` / `@5`;
> File and network Contracts use `std/…@3`. Loop extends an existing terminal;
> Retry repeats a pinned Call without adding a port. Earlier milestone names below identify when a responsibility
> landed; they are not current package versions or pending-work claims.
> Current limits are in [Loop](../spec/loop.md#16-deferred-by-name),
> [observability](../spec/observability.md) and [release qualification](release-qualification.md).

This document refines the responsibility table of
[`ARCHITECTURE.md`](../../ARCHITECTURE.md) to the component level: the
data forms that flow through the system, the Rust modules and types
that own them, the dependency direction between modules, and what the
current build implements. It adds no language or runtime semantics;
on any conflict,
[`docs/spec/`](../spec/README.md) wins. The
[C4 component view](03-rastok-components.svg) remains the coarse visual.
The [core data view](05-rastok-packages.svg) shows current owned types. The maintained
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

## 1. The data flow

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
typed journal records (INGRESS / TRANSACTION / EGRESS)
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
| `boundary::{Intent,SessionOpenIntent,SessionWriteIntent}` (open EGRESS) | `runtime` | dispatcher/bindings → exchange-specific `boundary::OutboundConnectorDriver` views | canonical Call or Session effect fields and already-resolved non-plan settings; each causal open/write record seals its emitting id, connector path and exact request/message digest before Driver effect, while settings remain execution material rather than a parallel causal field ([delivery-semantics.md §4](../spec/delivery-semantics.md); [identity.md §3](../spec/identity.md)) |
| `boundary::Completion` | `connectors` or the Lab's mock Driver | `runtime` (as an input event) | returns ordered ordinary port drafts plus one final typed `exit`, correlated by the internal `CallId` — never by the business key; a Driver only answers and never decides a timeout or mints journal records |
| `journal::Record` | **runtime mechanics only** | recovery, `golden::Trace` | Drivers and mocks supply events, never records; causal core and observation annotations kept separable ([identity.md §3](../spec/identity.md)) |
| `observation::RecordObservation` | semantic journal query | deterministic reports and disposable hosted projections | typed correlation is derived only from causal history, retained outside its seal, and never consulted by execution or replay ([observability.md §6–7](../spec/observability.md)) |
| `accounting::AccountingObservation` | read-only journal/schema inspection | individual measurement inspection | original exact samples, annotation id/unit and causal provenance; no grouping, pricing or subset arithmetic in M1 ([observability.md §7.1](../spec/observability.md)) |
| `diagnostics::Diagnostic` | every phase | entry points | stable code, spans, structured fields; accumulated values — never prints, never panics |
| golden projection | `golden::Trace` (project, seal, diff) | evidence compare | deterministic bytes: same scope, same bytes ([simulation-semantics.md §6](../spec/simulation-semantics.md)) |

## 2. The module map

```text
crates/rastok-core/src/       # the kernel — #![no_std] + alloc (split at M0.5)
├── datatype.rs, value.rs       # logical types, values, validation/conformance
├── schema_annotation.rs, data_policy.rs, accounting.rs
│                              # sparse policy, derivation, individual measurements
├── node.rs, envelope.rs, expr.rs, plan.rs
├── canonical_cbor.rs         # one bounded canonical byte discipline
├── canonical.rs              # canonical envelope/type/value registry
├── instance.rs               # identity, generation, policy and live lifecycle
├── json.rs, trivia.rs        # shared exact JSON and lexical-comment facts
├── deployment.rs             # typed boot/secret/privacy/price inputs, no environment reads
├── journal.rs, observation.rs # causal records plus retained typed correlation
├── boundary.rs               # host vocabulary, request framing, five traits
├── plan/                     # Dispatch, routing, composition, policy, validation, serialization
└── runtime/                  # TransactionPreparer, VM, Loop/Retry managers, Closing and recovery

crates/rastok/src/            # the std layer — re-exports the kernel
├── diagnostics.rs    # Diagnostic, Code, Severity, Phase, Span, Bag
├── compiler.rs       # facade : parse → check → hash → lower → validate
│   └── compiler/     #   lexer (cursor + lookahead), parser (lexemes only),
│                     #   syntax, hir (graph + lab), check, lower
├── contract.rs       # compiler contracts: node attributes plus tagged frontend
│                     #   framing/readers, all admitted through Registry
├── schema_projection/# strict JSON Schema, Avro, Arrow and OpenAPI adapters;
│                     #   path-specific loss and physical union mappings
├── openapi.rs         # deterministic project endpoint/type resolver
├── stdlib.rs         # the standard CATALOGUE instancing those contracts (PROCESS,
│                     #   declared connector names, built-in/deferred frontends)
├── golden.rs         # Trace, Divergence — projection, seal, named diff
├── recording/        # bounded diagnostic files, exact-record codec and read-only
│                     # recovery/UI; not the M2 durable deployment Journal
├── connectors/       # registry/global binder plus one module per Connector contract
│   ├── registry.rs   # exact-id factories and role-aware RuntimeBindings
│   ├── manifest.rs   # pre-load data-only Connector id/role capabilities
│   ├── process/      # Process: bind effective settings, then async Driver
│   ├── file/         # File: pure operation framing, then filesystem Driver
│   ├── http_client.rs, http_server.rs
│   ├── websocket_client.rs, websocket_server.rs
│   ├── websocket_protocol.rs # shared RFC 6455 message responsibility
│   └── network/      # private sockets/TLS/pumps/protocol codecs
├── conformance.rs    # Driver contract harness over the production seam
├── lab.rs            # run → ScenarioRun { report, records } ; mock Drivers
├── deployment_config.rs # hosted profile JSON and provider resolution
├── observability.rs  # bounded never-block structured-log projection
├── ui/               # optional loopback HTTP/SSE controller and embedded page;
│                     #   journal/inspection/accounting projection only
├── render.rs         # Sources + rustc-shaped diagnostic rendering
└── main.rs           # thin bin : check | run | replay | simulate | --resolve-open-api;
                      # optional --record, --json-logs and --ui
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
   `ScenarioReport`, `ValidatedExecutionPlan`. Names follow actual contracts;
   a future responsibility does not justify a placeholder type or module.
3. **Qualified paths give context; precision beats brevity.** A type
   never repeats its module name (`expr::Tree`, never `ExpressionTree`), but
   keeps a precise compound when the bare word would be too generic
   (`lab::ScenarioRun`, `golden::Divergence`) and keeps the
   corpus term even when it overlaps the module word
   (`plan::ExecutionPlan`, `plan::SemanticHash`). The `x::X` stutter
   stays idiomatic where the type *is* the module's whole concept:
   `journal::Journal`, `envelope::Envelope`.
4. **Cohesive facades.** Subdirectories group one contract or split a large
   responsibility behind its existing facade. Dispatch, routing, validation
   and recovery now have focused submodules; depth is not an API promise.
   A directory is never created for an unimplemented responsibility.
5. **One engine.** There is no simulator component: `rastok simulate`
   drives the same `runtime`, injects/advances through its public VM
   surface, and substitutes the `OutboundConnectorDriver` with the Lab's mock.

## 3. Component inventory

Columns: what the component owns, its interface stated as data in →
data out, the governing spec, and current status.

### 3.1 The compiler (`compiler`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **Lexer + parser** | one forward byte cursor and token lookahead in `Lexer`; complete host lexemes; byte-adjacent `IDENT!` frontend selection; specialized expression/JSON/verbatim/guest regions; spans and structural recovery | source bytes + frontend Registry → syntax tree + diagnostics | [grammar.md](../spec/grammar.md); D-2026-08-13-1; D-2026-08-14-1 | M1.3 B4 |
| **Unit assembly** | the deployed unit: multi-file resolution, lab-fragment merge, exactly-one-App, wiring by name, no shadowing | syntax trees → one resolved `Unit` | [grammar.md](../spec/grammar.md) scope rules; [simulation-semantics.md §2](../spec/simulation-semantics.md) | M0 (single unit) |
| **Desugarer** | the finite sugar table — afterwards there is one way to say each thing | resolved unit → core-form graph | [grammar.md](../spec/grammar.md) | M0 |
| **Type checker** | Node/Graph/Executor/Connector/Extension, one Dispatch per trigger, typed routes, explicit productibility/dispositions, instances, joins, policy and three qualified Loop hosts with a resolved host exhaustion terminal | source → checked `Hir` + diagnostics | [grammar.md](../spec/grammar.md); [types.md](../spec/types.md); [observability.md](../spec/observability.md); [instances.md](../spec/instances.md); [expressions.md](../spec/expressions.md); [extensions.md](../spec/extensions.md); [loop.md](../spec/loop.md); [effects-and-sagas.md](../spec/effects-and-sagas.md) | executable; qualified by the current identity inventory |
| **Canonicalizer** | the canonical byte form of the resolved HIR; the semantic hash | `Hir` → canonical bytes + `plan::SemanticHash` | [identity.md §1](../spec/identity.md) | M0, extended in M1 |
| **Lowering + plan gate** | owner/address tables, Operator Dispatch, Contract-defined Connector transitions, RouteRule/PhysicalEdge separation, ExtensionSpec, manager commands and sparse policy; Graphs carry no application handlers | `Hir` → `plan::ValidatedExecutionPlan` | [identity.md](../spec/identity.md); [observability.md](../spec/observability.md); [instances.md](../spec/instances.md); [execution-model.md](../spec/execution-model.md); [extensions.md](../spec/extensions.md); [plan-package.md](plan-package.md) | categorical plan executable; hostile validation in core |
| **Tagged frontends** | registered name, framing and reader policy; built-in JSON/text/base64 decoding; deferred guest/data compiler identity; the host never tokenizes an owned body | `IDENT!` region → typed value, named deferral, or future compiled guest program | [expressions.md](../spec/expressions.md); [stdlib.md §5](../spec/stdlib.md) | B4 framing/readers; guest execution guarded |

The facade returns `Result<Compilation, diagnostics::Bag>` with
`Compilation { plan, symbols, hash, warnings, labs, source_definitions,
scenario_definitions }`. The two source side tables support redacted inspection
and never enter plan identity. **A plan with errors is
unrepresentable** — warnings ride the success value, errors preclude
it, and `plan` already has type `ValidatedExecutionPlan`. Checked Executor
semantics are an algebraic type:

```text
hir::NodeSemantics = Operator
                   | Connector { contract: ContractId, roles, exchange, attributes }

Contract roles and exchange profile select admission and dispatch.
Idempotence is a separate validated guarantee used by Retry.
```

Graphs have their own checked scope tree, not an Executor variant. Parsed
`GraphItem`s may still carry invalid forms for named diagnostics; checking and
independent package validation enforce legal attributes, required Connector
settings and the absence of Graph/Connector application code. The syntax tree
is not a proof of those invariants. Connector roles, not an effect class,
distinguish inbound sources from outbound callers. HttpServer executes; Timer and Calendar
remain declared-only and reject before HIR construction
([effects-and-sagas.md §2.1–2.2](../spec/effects-and-sagas.md),
[stdlib.md §2](../spec/stdlib.md), [loop.md](../spec/loop.md)). The checker reads Connector
contracts from `stdlib` (§3.7) through the `contract` metamodel,
never from Driver code — and it INJECTS every omitted `Defaulted`
attribute into the effective HIR (D-2026-08-12-1), so the plan
carries effective attributes only and Drivers never reintroduce a default.
D-2026-08-18-1 adds one explicit distinction inside that effective row: a
plan-resolved word/value versus a contract-authorized dynamic expression. The
contract declares its latest permitted site; the effective row transports the
earliest actual boot/instance/invocation site selected by checking. `Hir` is the
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
| **`expr`** | the expression compiler/evaluator over core `DataType` and `Value`; its evaluator-only carrier may additionally hold the lazy payload witness | (text, `Signature`) → `Tree`; (`Tree`, `Bindings`) → evaluation result; (`Tree`, boot constants) → core value | [expressions.md](../spec/expressions.md); [execution-model.md §5](../spec/execution-model.md) (no ambient time) | core type/value authority; `media_type(text)` is executable |

`expr` never sees an envelope, Node, or placement boundary, and owns its own
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
extensions and divergences are listed in `expressions.md`. V7 assigns the
resolved constructor/narrowing forms their canonical package ids.

### 3.3 The executable plan (`plan`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`plan::ValidatedExecutionPlan`** | private validated plan with total ports, owners, recursive Dispatch, route selection, Contract profiles, Extensions, Loop proof/host terminals and sparse policy tables | produced by `compiler` or `plan::decode`; consumed by `runtime`, `lab`, and `plan::encode` | [types.md](../spec/types.md); [observability.md](../spec/observability.md); [identity.md](../spec/identity.md); [instances.md](../spec/instances.md); [extensions.md](../spec/extensions.md); [plan-package.md](plan-package.md) | current format executable; unknown versions/tags refuse |
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

The current build transports exact `BootReference` values for HttpClient
`url = env(...)` and `auth_bearer = secret(...)`. `deployment::BootBindings`
supplies typed deployment values; hosted `deployment_config` reads profile JSON
and the selected secret provider. The run header attests references and versions,
not clear credentials. General boot `val`s and other secret consumers remain
named gates; the evaluator never reads the environment.

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
explicit: `Vm::recover_from_journal` and `recover_with_bindings` rebuild a
candidate VM from retained records before replacing live state. They do not
re-execute producers or perform physical I/O. “RecoveryFold” names the design
responsibility, not a public Rust type. The Lab runner and visual debugger use
the same VM surface; authored Lab crash/restart actions remain deferred.

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **Scheduler** | run-to-completion firing; the single dated-event queue — the **call budgets, created at DELIVERY** so queue time counts, referenced by `(edge, envelope id)`, resolved reserve→resolve→ack so a refused expiry replays (a timeout is the scheduler's dated event, never a Driver answer); virtual-time `advance`; drain-to-stable | plan + ordered events → transaction firings | [execution-model.md §1, §5–6, §8](../spec/execution-model.md) | M0 (single worker) |
| **Mailboxes** | per-instance-reference FIFO order, arrival-order capture (the race outcome is journaled), bounded capacity | envelopes → ordered delivery within one address generation | [execution-model.md §2–3](../spec/execution-model.md) | M0 base; M1.4 R3 hierarchical rebase landed |
| **TransactionPreparer** | the atomic node transaction: consumed input, state delta, emissions, and records commit together; scratch isolation while a handler runs | firing → transaction commit + emissions | [execution-model.md §1](../spec/execution-model.md) | M0 |
| **EmissionBuffer** | per-port emission identity in text order (`emission_index`) | handler body → ordered emissions | [execution-model.md §4](../spec/execution-model.md) | M0 |
| **Boundary completion** | one correlated completion closes a Call and carries ordered ordinary output drafts plus one final typed `exit`; the VM mints and commits all output envelopes atomically. Outbound Calls consume a delivery and close an EGRESS record; inbound Calls close their committed root through `l:reply`: fixed ack consumes no cursor, while a graph response consumes its one already-journaled delivery. The orthogonal Session path journals opening and individual write obligations, then publishes one terminal exit. | completion drafts or session facts → causal records and routed envelopes | [types.md](../spec/types.md); [m1.3 backlog](m1.3-type-system-backlog.md); [M1.6 backlog](m1.6-http-websocket-backlog.md) | M1.3 B7 outbound Call; M1.6 H1 Session fold; H5/H5.2 generic inbound Call |
| **Instance Closing** | durable close-scope arbitration; successor-generation admission; bounded drain, canonical cancellation and settlement; residual cursor retirement; inner-first stop and state reclamation | `closing` policy + lifecycle cause + journal fold → terminal exits and `l:stop` | [instances.md §7](../spec/instances.md); [execution-model.md §2.1](../spec/execution-model.md) | M1.4 R5 |
| **`GuestRunner`** | the existing runtime seam future guest engines implement: `run(&[u8], value, budget)` — pure, deterministic, budgeted, inside the node transaction; the bytes become a typed/versioned `GuestProgram` only when that slice lands | opaque program bytes → value or typed error | [stdlib.md §5](../spec/stdlib.md) | seam only; no engine/program type yet |
| **`deployment::BootBindings`** | exact environment values, opaque SecretName/version bindings and privacy/price attestations; clear material stays at the Driver boundary | plan + typed deployment profile → bound settings and run attestation | [observability.md](../spec/observability.md) | M1.8 A4, narrow HttpClient sites |

### 3.5 The boundary (`boundary`) — host interfaces and explicit time

Every nondeterministic thing is represented at one of these host
boundaries, and what crosses into causality is recorded — the journal's
*world-half* is the record of it. The current Lab substitutes outbound Call
Drivers, the exchange conformance suite supplies Event/Session Drivers, and
H5 supplies the generic inbound Call fold, H5.1 a live bounded H1/H2 ack
Driver and H5.2 the generic graph-response handoff; direct
scenarios may still inject drafts through `Vm::inject`, and virtual time
advances through `Vm::advance`. Hosted code maps one civil origin plus a
monotonic timer to that explicit input; Lab and bare-metal callers may provide
their own origin. The evaluator never reads either clock. `Supervisor` remains
a declared host seam rather than an object the M1 VM consumes directly.

| Interface | Owns | Real side | Lab side | Spec | Status |
|---|---|---|---|---|---|
| **`InboundConnectorDriver`** | one retained `IngressProposal` — draft, provenance, sequence request and optional Session framing — until the runtime answers accepted-after-commit or rejected; pending admission makes no callback and therefore applies bounded backpressure. It never births an envelope: only the journal's accepting transaction mints root/exchange identity. Call bindings expose retained acknowledgement/response; Session bindings correlate each admitted message to the committed logical `SessionId`. | live HttpServer and WebSocketServer over H1/H2/H3 | Contract-owned fake Drivers; direct scenario injection remains available through `Vm::inject` | [envelope.md §1.1, §2](../spec/envelope.md); [delivery-semantics.md §3, §4.4.1, §4.5, §6](../spec/delivery-semantics.md); [simulation-semantics.md](../spec/simulation-semantics.md) | M1.6 H1 generic protocol; H5 Calls; H6 Sessions |
| **`OutboundConnectorDriver`** | categorical exchange view: `CallConnectorDriver` launches journaled `Intent`s and answers by `CallId`; `SessionConnectorDriver` launches journaled open/write intents and reports by `SessionId`/`SessionWriteId`. Cancellation/timeout remain runtime policy and no Driver writes causal records. | Process/File/HttpClient Calls; WebSocketClient/Server Sessions | Contract-specific Call and Session mocks plus generic conformance batteries | [delivery-semantics.md §4](../spec/delivery-semantics.md); [execution-model.md §6.1, §6.6](../spec/execution-model.md); [M1.6 backlog](m1.6-http-websocket-backlog.md) | Call semantics M1.3 B7; role name M1.5 C0; live Sessions M1.6 H6 |
| **Logical-time input** | `Vm::boot_at` fixes an origin and `Vm::advance(journal, driver, target)` advances one deterministic timeline. A hosted runner uses one wall-clock sample plus monotonic elapsed time; a microcontroller with no RTC may use zero or a supplied origin. `now()` observes the firing's fixed instant and seals it only when used. | hosted time source | scenario origin + `advance` | [execution-model.md §5, §8](../spec/execution-model.md); D-2026-08-28-5 | target-driven VM; handler observation current |
| **`Supervisor`** | process lifecycle: boot, crash, restart; recovery must never require a clean shutdown | host lifecycle | not built; fault steps remain guarded | [delivery-semantics.md §4.4](../spec/delivery-semantics.md); [simulation-semantics.md §5.4](../spec/simulation-semantics.md) | declared seam; current wiring boots only |

The **`Dispatcher`** (owned here) is the loop between the journal and
Outbound Connector Drivers: it takes EGRESS records the VM opened, drives
the selected binding, and settles answers through
`journal::close_egress(call_id, …) → Won | AlreadyClosed`. Before
any closure, the **`CompletionGate`** judges every output against the resolved
port contract — a refused reply never becomes
a causal fact: it returns in `PumpBatch::rejected` with its reason,
and the call stays open, honestly settleable by the timeout. Only a
`Won` closure becomes an input event for the VM; `AlreadyClosed`
degrades the late arrival into a discarded observation — the
anti-ghost rule
([execution-model.md §6.3](../spec/execution-model.md)) as a typed
race, not a convention. `PumpBatch` returns every non-causal
observation — discarded, uncorrelated, rejected — nothing silent.

Ownership rules that the types must make unrepresentable:

- an Outbound Connector Driver **answers or stays silent** — it never returns a timeout
  (the scheduler's dated event decides that) and never returns
  `OutcomeUnknown` (recovery or bounded Closing supervision mints it);
- call identity is the internal `CallId` — a **deterministic
  derivation of the canonical call fields**, never a new serialized
  causal field; the business/idempotency key names the *operation* for
  retries and cooperative external-side dedup — the two are never
  conflated;
- an inbound envelope is **born inside the accepting transaction**:
  the Inbound Connector Driver supplies a draft; only the journal can mint a root id
  (the envelope is immutable and `meta.sys` runtime-owned);
- Drivers and mocks produce **events**; the runtime mechanics alone
  write records — a mocked call still gets its EGRESS record because
  the VM writes it before any Driver, real or scripted, is invoked;
- response and timeout can race, but **exactly one wins** the closure;
  the loser is recorded as a **discarded observation**, never a causal
  fact.

### 3.6 The journal (`journal`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`Journal`** | the typed logical records and their order plus retained typed `RecordObservation` correlation outside the causal seal; `MemJournal` in-memory first | **semantic operations**, not bare appends: `prepare_ingress` mints the final root envelope without consuming its sequence; `commit_ingress` atomically advances that sequence with producer-sealed routes; `instance_generation_for_admission` folds lifecycle history; `commit(Transaction)` verifies firing/parent/route/closing generations; `open_egress` is journal-before-effect; `close_egress` and `abandon_egress` return `Won \| AlreadyClosed`; `retained_observations` feeds reports/export without exposing physical layout; individual annotated values are read without aggregation or a second ledger | [delivery-semantics.md §1, §4, §6](../spec/delivery-semantics.md); [observability.md §6–7](../spec/observability.md); [envelope.md §2, §5](../spec/envelope.md); [instances.md](../spec/instances.md) | M0 base; M1.4 R5 closing; M1.8 observation and accounting planes |
| **Id derivation** | derived envelope and record ids — zero per-envelope randomness | recipe inputs → `envelope::Id` | [envelope.md §2–3](../spec/envelope.md) | M0 |
| **Spatial/Loop recovery** | `runtime/vm/recovery` reconstructs generations, deltas, pending deliveries/cursors, Calls/Sessions, retained original deadlines and r2 runs/commands; physical effects are never replayed by reconstruction | journal prefix + optional pure Contract projections → recovered VM, or unchanged original VM on refusal | [delivery-semantics.md](../spec/delivery-semantics.md); [loop.md](../spec/loop.md) | R7–R9 executable; no disk backend |
| *(golden projection)* | moved to its own `golden` module as built — see §3.6b | | | |

### 3.6b The golden evidence (`golden`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`golden::Trace`** | the canonical causal projection and blake3 seal, encoded/decoded through shared canonical CBOR; deadline identities are causal, the separately unsealed deadlines is not part of this projection | records → deterministic bytes; stored golden → structural re-read | [identity.md §3](../spec/identity.md); [simulation-semantics.md §6](../spec/simulation-semantics.md) | M0.5, extended causal registry |
| **`golden::Divergence`** | the first meaningful difference, by record index and NAMED key path — a broken seal explains itself, never a hex dump | two traces → divergence or none | — | M0 |

The module owns both directions of the **trace projection** and uses the
shared canonical CBOR reader/writer, so a mismatch diffs structurally
against the STORED golden; the workflow (compare, explicit `--bless`)
is driven by the lab's returned records, the golden tests, and `main`.

`IngressRecord` carries the full root recipe — the opaque
`IngressEnvelopeProvenance` (wrapping exactly the normative
`(connector_path, replica)` pair) plus the sequence — so a record autonomously
reconstructs its root id ([identity.md §3](../spec/identity.md)). The
projection lives here because it also serves production replay and
cross-runtime conformance; the golden *workflow* (compare, explicit
bless, diff rendering) is driven by `lab` and `main`.

### 3.7 The stdlib contracts (`stdlib`)

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`contract::ConnectorContract` / `ContractCatalog`** | the compiler-facing contract sheet: canonical `ContractId`, required roles, one `PortContract` table, common attributes, optional profile selector, and effective presence/default/domain/evaluation-site rules. The catalogue is keyed by id while source symbols remain a checker concern; two symbols cannot alias one id | resolved by the checker into self-contained plan rows; **never depends on Driver code and never travels as a catalogue** | [stdlib.md §1–3](../spec/stdlib.md); [instances.md §3](../spec/instances.md); D-2026-08-22-1 | M1.5 C1 |
| **`contract::FrontendContract`** | an ordinary source name plus its registered framing/reader policy. Built-in JSON/text/base64 execute; guest/data names retain a specific framing contract and named deferral until their compiler lands | consumed by parser and checker; never serialized by itself | [expressions.md](../spec/expressions.md); [stdlib.md §5](../spec/stdlib.md); [identity.md §1](../spec/identity.md) | M1.3 B4 |

This module answers "how does the compiler know what `Process` is
without linking its Driver": the contracts are data, Drivers implement
them, and the checker validates declarations against the spec sheet
alone.

### 3.8 Connectors and guest engines (`connectors`, `guest_engines`)

The effectful and the pure halves of extension, on mirrored seams: a
`connectors/*` implementation satisfies one or both boundary Driver roles; a future
`guest_engines/*` engine implements `runtime::GuestRunner` and will
pair with a compiler frontend registered only when the first real guest
implementation lands. No placeholder `GuestFrontend` type exists in the
current compiler. A guest program with I/O belongs behind a Connector,
whatever its execution format.

| Component | Owns | Spec | Status |
|---|---|---|---|
| **`connectors::DriverRegistry`** | exact `ContractId` → implementation factory mapping and role-availability checks; duplicate ids refuse | [decisions.md D-2026-08-22-1](../spec/decisions.md) | M1.5 C1 |
| **`connectors::DriverManifest`** | data-only, canonically ordered stable id/role claims readable before implementation code loads; no factory, artifact locator, ABI or trust policy | [decisions.md D-2026-08-22-1](../spec/decisions.md); [connector-drivers.md](connector-drivers.md) | M1.5 C6 |
| **`connectors::RuntimeBindings`** | the pre-VM association of every runnable inbound and callable outbound Connector with one bound Driver object; exact role-view proof; Node-path and Call routing without concrete contract matching | [decisions.md D-2026-08-22-1](../spec/decisions.md) | M1.5 C2 |
| **`connectors::ProcessFactory` / `Process`** | the explicit `std/process@4` factory and real job-mode Outbound Driver: pre-binds every effective setting before the VM/journal exists, spawns asynchronously, captures under one aggregate budget, and supports `lines` / `jsonl` / `bytes` | [stdlib.md §3.3](../spec/stdlib.md) | M1.2 behavior; contract-specific module C0; registry binding C2 |
| **`connectors::ProcessServiceFactory`** | `std/process@5`, one resident Session per exact instance; framed streaming, bounded writes, Unix supervision, no application RPC | [stdlib.md §3.3](../spec/stdlib.md) | [Resident qualification](process-service.md); pure Lab substitution and no package move |
| **`stdlib::FILE` contract** | compiler sheet and Registry-free core mirror for `std/file@3`: `read`/`replace`, exact pins and `FileExit`, invocation-bound path, read budget | [stdlib.md §3.7](../spec/stdlib.md); D-2026-08-22-1 | M1.5 C3 |
| **`connectors::FileFactory` / `File`** | explicit physical-root binding and real one-file Outbound Driver: seals mode/path/budget-or-payload before open; performs complete bounded reads or native atomic replace after open; cooperates with generic cancel/close-once/replay rules | [stdlib.md §3.7](../spec/stdlib.md); D-2026-08-22-1 C4 checkpoint | M1.5 C4 |
| **Network contract sheets** | exact `std/http-client@3`, `std/http-server@3`, `std/websocket-client@3`, and `std/websocket-server@3` roles/exchanges, codec-selected ports, phased settings, lossless HTTP metadata and portable terminal unions; the Registry-free mirror refuses forged rows without consulting the compiler catalogue | [stdlib.md §3.1–3.2, §3.8–3.9](../spec/stdlib.md); D-2026-08-25-2 | M1.6 H2 |
| **flat HTTP/WebSocket Connector modules** | `http_client`, `http_server`, `websocket_client` and `websocket_server` own contract-specific binding; `websocket_protocol` alone owns shared message framing. Every Contract resolves complete settings before external work and exposes only generic Call/Session views to the registry. | [M1.6 backlog](m1.6-http-websocket-backlog.md) | M1.6 H2–H7 complete |
| **private `connectors::network` seam** | feature-selected bounded synchronous/async pump; Hyper/Rustls H1-H2, quiche/BoringSSL H3, shared trust bytes and bounded Tungstenite message machine; all runtime/library types stop below the Driver boundary | [connector-drivers.md](connector-drivers.md); D-2026-08-26-1; [M1.6 exit](m1.6-exit-report.md) | M1.6 H3 engine through H6 live client/server/Session execution |
| **`connectors::http_client` / hosted HttpClient** | exact pre-open HTTP request descriptor; typed standard/custom request headers; bounded `CallStart` admission; H1 origin-form, pooled H2 and verified quiche H3; visibility-safe fallback; codec/body/header gates; lossless and typed response metadata plus portable exits | [stdlib.md §3.2](../spec/stdlib.md); D-2026-08-26-2/-3 | M1.6 H4/H4.1 |
| **`connectors::http_server` / hosted HttpServer** | pre-bound ack/respond routes; bounded H1 connections, H2/H3 streams, retained byte reservations, commit-before-accept proposal decisions, correlation, deadline and close-once terminal observations | [stdlib.md §3.1](../spec/stdlib.md); D-2026-08-26-6 through D-2026-08-27-1 | M1.6 H5 complete |
| **hosted WebSocket Client/Server** | one logical Session over H1 Upgrade or H2/H3 Extended CONNECT; bounded complete-message admission and writes, portable terminal exit, server multi-session correlation and client one-session-per-instance | [stdlib.md §3.8–3.9](../spec/stdlib.md); D-2026-08-27-2 | M1.6 H6 complete |
| **`connectors::assembly::hosted_registry`** | deterministic feature-selected registration of Process, optional File and linked network Contracts; manifest projection; no listener or effect during assembly | [connector-drivers.md](connector-drivers.md); D-2026-08-27-3 | M1.6 H7 complete |
| **Connector conformance profiles** | one role-aware battery over `DriverRegistry`/`RuntimeBindings`; exact resolved terminal union, silence, typed completion coverage, correlation, journal-before-effect, close-once and explicit unprovable cases | [stdlib.md §1](../spec/stdlib.md); D-2026-08-22-1 C5 checkpoint | M1.5 C5 (Process + File) |
| **Catalogue remainder** (`Timer`, `Calendar`, `Memory`) | names declared so source refuses explicitly; contract sheets and Drivers land by slice | [stdlib.md §3](../spec/stdlib.md) | guarded |
| **Future jq/sql engines** | pure guest engines under the six-clause contract; no `guest_engines` module exists yet | [stdlib.md §5](../spec/stdlib.md) | guarded |

### 3.9 The lab (`lab`)

A Driver set plus the production machine — never a second engine.

| Component | Owns | Interface | Spec | Status |
|---|---|---|---|---|
| **`lab::run`** | driving the production `Vm` through each checked scenario: inject, advance, settle, expectation windows over the machine's observation log, the three quantifiers; every uncovered call, faulting predicate or guard, and unsettled machine is a named failure | `hir::Lab` + plan → `Vec<ScenarioRun { report, records }>` — the records feed the golden workflow | [simulation-semantics.md §2, §5–6](../spec/simulation-semantics.md) | M0 |
| **The scripted world** | as built, leaner than sketched: the *fixture* IS the scenario's steps (injections through `Vm::inject` — no `Fixture` type); the *virtual timeline* IS `Vm::advance` (no separate time-source type); explicit mock factories bind by the same exact Contract id/roles and reuse each Contract's pure request protocol — replies only, the VM still writes every EGRESS; the fault injector stays `next` | the scripted external world | [simulation-semantics.md §2, §5](../spec/simulation-semantics.md) | M0; multi-Contract registry parity M1.5 C5 |
| **Coverage** | the statically enumerable failure-space checklist | plan + scenarios → coverage report | [simulation-semantics.md §7](../spec/simulation-semantics.md) | later |
| **Unsupported surfaces** | `integration`, `theorem`, `experiment`: parse, then refuse explicitly | reachable syntax → `unsupported` diagnostic | [simulation-semantics.md §10–11](../spec/simulation-semantics.md) | guarded |

Scenarios are parsed by the same `compiler` front end as graphs — the
simulation grammar is the graph grammar mirrored — so the lab owns no
second parser and no second engine.

### 3.10 Cross-cutting (`node`, `envelope`, `diagnostics`, `main`)

| Component | Owns | Spec | Status |
|---|---|---|---|
| **`node`** | shared path, port, Connector `behavior`, trigger and closed `Extension` vocabulary; Executor is a semantic category, not a public trait | [grammar.md](../spec/grammar.md); [extensions.md](../spec/extensions.md); [effects-and-sagas.md](../spec/effects-and-sagas.md); [stdlib.md §2](../spec/stdlib.md); [envelope.md §3](../spec/envelope.md) | Current shared vocabulary |
| **`envelope`** | `Envelope { meta, data }` with four `meta.sys` fields (`id`, `parent_id`, `origin_id`, `graph_version`), typed `app`/`ext` metadata, and encoded/materialized DATA; Loop context and manager commands remain journal facts rather than new envelope fields | [envelope.md](../spec/envelope.md) | Current envelope, without `role` |
| **`diagnostics`** | stable codes, severity, the registry's **phase vocabulary** (`parse`, `resolve`, `check`, `plan`, `run`, `replay`, `sim`), multi-span, structured fields; a check ships with its code, its negative test, and its registry row in the same change | [diagnostics.md](../spec/diagnostics.md) | M0 |
| **`main`** | the `check` / `run` / `simulate` verbs, wiring (Drivers, dispatcher, stdlib contracts), rendering; owns nothing semantic | — | M0 |

## 4. Dependency direction

```text
main (bin)
   │
   ├──▶ lab (run + mock Drivers) ───────────────┐
   │                                            ├──▶ boundary contracts ──▶ runtime
   └──▶ run wiring (+ stdlib) ──────────────────┘        ▲                     │
                                                         │                     ▼
        connectors ── implement ─────────────────────────┘                 journal
        future guest engines ── implement GuestRunner ▶ runtime               ▲
        boundary::Dispatcher ── drives open EGRESS facts, closes ───────────────┘

compiler ──▶ plan (the execution plan) ◀── interprets ──────────────────── runtime
    │    └─▶ stdlib (contract sheets)
    ▼
   expr  (signatures in, trees out; shares only canonical/json/trivia foundations)
```

Rules, in force from the first commit:

1. **`runtime`** consumes the plan, core types/expressions, deployment/policy
   inputs, boundary contracts and the journal — never a concrete hosted
   Driver implementation, the Lab, or the compiler. The M0 debt is closed:
   `boundary::prepare_request` creates one `PreparedRequest`, so the
   digest and transported bytes are the same sealed value without a
   VM→`connectors` dependency.
2. **`expr`** depends on no graph/runtime concept; it consumes core
   `DataType`/`Value` and shares canonical, JSON and trivia primitives.
3. **Driver/guest implementations** (`connectors`, future `guest_engines`, the Lab's boundary
   implementations) implement traits. Hosted CLI, Lab and conformance assembly
   bind concrete implementations; the core consumes their abstract role views.
   The boundary dispatcher uses the `Journal` trait, never a concrete store.
4. **`stdlib`** is declarative data over the base vocabulary; the
   checker and `main` read it; it depends on no Driver.
5. **`lab`** consumes compiler output, the public VM, boundary/Driver bindings,
   and journal records. It never patches runtime internals or implements a
   second execution engine.
6. **`node`, `envelope`, `diagnostics`** are shared foundations for the
   layers that need them; `node` is leaf-like, `envelope` builds on it,
   and diagnostics stays in the compiler/std crate rather than entering
   the `no_std` runtime vocabulary.
7. **`golden`** reads journal records, not the VM.

The `rastok-core` / `rastok` split now makes the most important frontier
mechanical: the `no_std` kernel cannot import the compiler, Drivers, Lab,
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
- **Opaque ingress provenance.** `boundary::IngressEnvelopeProvenance` wraps
  exactly the normative `(connector_path, replica)` pair
  ([identity.md §3](../spec/identity.md)) but stays opaque: M0 code
  never assumes the replica component is a constant it may inline. The value
  is provenance shared by inbound roots, not an address or submission id.
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
  unsupported `env()` / `secret()` sites refuse explicitly; only the two
  declared HttpClient sites bind through M1.8 deployment values. Silence is the only forbidden outcome.

## 6. M0 coverage map

The slice stages of [`m0-slice.md`](m0-slice.md), in module terms:

| Slice stage | Exercised |
|---|---|
| Parse | `compiler` (parse), `diagnostics` |
| Check | `compiler` (assemble, desugar, check vs `stdlib` contracts), `expr` (compile) |
| Plan | `compiler` (canonicalize, lower), `plan::Manifest` |
| Step | `runtime` (Scheduler, Mailboxes, TransactionPreparer, EmissionBuffer, error routing) |
| Handle | `boundary::Dispatcher` + `boundary::OutboundConnectorDriver` behind `CompletionGate`; `connectors::Process` (App) or the Lab mock (Lab) |
| Record | `journal` (semantic operations, id derivation) |
| Compare | `golden` projection, seal, and diff; `lab`-returned records; `main --golden/--bless` |

Historical M0 exclusions (not the current status above): `journal::RecoveryFold` and the lab's fault injector
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
  the executable subset. Their published transport is format v7.
- **Compiler interludes:** `contract` is the sole node-attribute
  metamodel; defaults are injected before identity, and the parser is a
  forward lexeme-stream parser. The lexer owns cursor/lookahead and
  delegates expression, JSON, verbatim, and guest regions without rewind.
- **M1.3 foundation:** one complete strict-profile `DataType`/`Value`
  ownership, validation and recursive conformance authority serves plans,
  state, metadata, expressions and VM doors. B2 adds a total effective port table,
  with declared/inferred origin, and makes unresolved/conflicting constraints
  typed refusals. B3 renames the consumer to
  `expr` and adds deterministic typed narrowing. B4 selects every registered frontend through byte-adjacent
  `IDENT!`, restores scalar JSON and keeps frontend framing out of the host
  grammar. B5-B7 land uniform Connector ports, tagged construction and
  multi-output completion. B8 publishes the complete v7 registry and total
  effective port table. B9 implements strict JSON Schema/Avro/Arrow
  projections, removes historical v6 type/expression scaffolding, and closes
  unchecked `variant` structural access.
- **M1.4 rebase:** D-2026-08-18-1 replaces the unreleased P0-P3 surface and
  transport with Node-owned `instance.key`, independent `lifetime`, private
  Node state, hierarchical logical addresses, and journal-derived instance
  references. R1 checking, categorical v9 owner transport (retained through
  v12), R3
  hierarchical routing/mailbox/state/cursor proofs, R4's five lifecycle
  profiles, and R5's bounded Closing protocol have landed. The reference VM remains synchronous and
  one-worker.
- **M1.5 closure:** Process and File bind through one registry, conformance and
  Lab protocol path. `DriverManifest` proves the pre-load id/role seam without
  freezing discovery or ABI, and [connector-drivers.md](connector-drivers.md)
  is the contributor path. Later M1 slices have since landed Loop, policy and
  accounting, and the optional `--ui` debugger in the order recorded by
  [`m1-slice.md`](m1-slice.md).

The R3–R9 rework and v17 amendment add the current load-bearing components:

- `plan::{Dispatch,RouteRule}` and independent scope/type/composition validators;
- `runtime::TransactionPreparer` with one selected Dispatch leaf and ordered multi-emission;
- `journal::loop_protocol` plus `runtime/vm/loop_manager`: distinct command
  production/consumption, exact admission identity, quiescence, fuel and delay;
- `runtime/vm/recovery`: spatial/exchange/deadline/Loop/Retry reconstruction from a prefix;
- `LoopSpec::expired`: a resolved existing terminal and extended type, never a
  contributed port or a VM comparison of the spelling `err`;
- hosted `ui`: recorded simulation transitions, source/decision/causal
  explanations and policy-aware DATA inspection. It never derives execution authority
  from an exported observation or invents a spatial manager edge.
