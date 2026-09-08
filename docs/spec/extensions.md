# Node, Graph, Executor, Connector, and Extension

> **Status (2026-09-08): normative categorical v17.** Loop's host-terminal
> amendment is D-2026-09-08-1. The underlying execution model and
> independent validation are qualified in [R9](../architecture/rework-r9-qualification.md).
> Unsupported Extension profiles refuse by name before effects.

This document owns Rastok's structural and extension taxonomy. Loop's temporal
protocol lives in [loop.md](loop.md); Saga's future contract lives in
[effects-and-sagas.md](effects-and-sagas.md).

## 1. Node has two structural forms

```text
Node
├── Graph
└── Executor
    ├── Operator
    └── Connector
```

| Concept | Meaning |
|---|---|
| `Node` | named structural owner with ports, instance, lifetime, Closing, and Extensions |
| `Graph` | typed boundary, lexical child Nodes, wiring, and immutable owner values |
| `Executor` | Node whose bounded activations produce atomic journal transactions |
| `Operator` | Executor whose transitions are described by DSL handlers and Dispatch |
| `Connector` | Executor whose transitions are described by one versioned Contract and typed Driver events |
| `Extension` | separately validated protocol attached to one Node owner |
| `Manager` | journal-derived runtime agent of a temporal Extension; not a Node or port |

A Graph owns no application mailbox, `var`, handler, or application
transaction. The generic lifecycle controller still commits `l:start`,
`l:closing`, and `l:stop` transaction records attributed to the Graph owner;
those records are instance facts, not Graph code.

Operator and Connector share the mechanics of execution: typed executable
ports, a bounded activation mailbox per instance, semantic state, transactions,
state deltas, ordered emissions, route preparation, capacity proof, and atomic
commit. They differ in the provenance of their transition. This semantic
category does not require a public Rust trait.

A Connector's Contract/exchange/cursor state is semantic, belongs to the core,
and is recoverable from the journal. Its Driver's socket, process, file handle,
pool, and library state are physical and reconstructible. A Driver supplies a
typed event; it cannot write semantic state or the journal directly. A
Connector has no DSL handler or author-declared `var` in v16.

## 2. Source forms

```rastok
graph app : App { ... }
graph refinement : Loop { ... }
graph agent : App + Loop { ... }
operator normalize { ... }
operator countdown : Loop { ... }
connector api : HttpClient { ... }
connector pay : HttpClient + Loop { ... }
```

The declaration keyword fixes structure:

- `graph` is a Graph;
- `operator` is an Operator Executor;
- `connector` is a Connector Executor and requires one Contract symbol.

The annotation contains the Connector's Contract when the declaration is a Connector and zero
or more Extensions. `App`, `Loop`, and `Saga` are Extensions, not Node types.
Contract is an object owned by a Connector, not a sixth branch of the Node
ontology. Its exact semantic identity is `<namespace>/<name>@<major>` and is
transported as `ContractId`.
Graph and Operator need no redundant type symbol. Historical explicit
`: Graph` and `: Operator` spellings refuse with a migration diagnostic rather
than surviving as aliases.

The checker resolves each symbol from a closed registry. Unknown, duplicate,
misplaced, or incompatible symbols are `E-NODE-ANNOTATION`. A recognized
Extension whose complete execution gate has not landed is
`E-CHECK-DEFERRED`; it never reaches an executable plan.

## 3. Closed Extension vocabulary

| Extension | Owner | Runtime meaning | Contributions |
|---|---|---|---|
| `App` | root Graph, exactly once | deployed public boundary | deployment-root identity; no manager |
| `Loop` | admissible Operator, Graph, or outbound Call Connector | finite temporal recurrence | qualified attributes, `Loop.state`, three commands, host terminal roles/types; no port |
| `Saga` | Graph, future gate | journal-folded compensation | future `Saga.compensate` protocol |

One source list is a set. Compatible Extension order is non-semantic:
`App + Loop` and `Loop + App` lower identically. The canonical plan sorts by
closed Extension tag. Incompatible contributions fail `E-EXTENSION-CONFLICT`;
there is no “first wins” rule.

An image which cannot execute a transported Extension refuses before any
effect with `UnsupportedExtension`. It never ignores the Extension. This keeps
one portable plan format while allowing hosted and reduced runtime images.

## 4. What an Extension may contribute

An Extension may contribute any of these, but need not contribute all:

| Contribution | Example |
|---|---|
| qualified attributes | `Loop.fuel`, `Loop.entry` |
| read-only bindings | `Loop.state` |
| admission constraints | finite host, unique recurrent input |
| manager commands | `Loop.next`, `Loop.again`, `Loop.done` |
| host port roles/types | Loop extends existing `in`, `out`, `err`, or the Call terminal `exit` |
| typed ports | future `Saga.compensate` |
| journal fold | Loop run/application state |

Every contribution is gated by the Extension. A qualified Loop name without
`: Loop` is an error. Duck typing from a coincidental port or attribute name is
forbidden.

Loop contributes no port. `Loop.expired = failed` selects an existing terminal;
it is an attribute, not a port. Productible exhaustion requires an authored
Operator/Graph union or the extended Call terminal pin accepting `LoopExpired`.

Commands are not ports. A temporal manager is addressed by its Extension owner
instance and receives journaled bounded commands. It is absent from the spatial
DAG and cannot be a source or destination of `PhysicalEdge`. An Extension port,
when one exists, is an ordinary typed port and follows the same routing and
explicit-destination rules as any other output.

## 5. Extension admission test

Extensions are intentionally scarce. A new proposal is admitted only if all
five tests pass:

| # | Test | Required answer |
|---:|---|---|
| T1 | runtime layer | requires a real state machine/scheduler beyond an existing Node, instance, Connector, or Extension |
| T2 | dedicated vocabulary | owns attributes, bindings, commands, constraints, or ports meaningless outside its scope |
| T3 | active orchestration | changes flow, timing, recurrence, or compensation rather than only observing it |
| T4 | non-redundancy | ordinary ports, Dispatch, state, instance, lifetime, and wiring cannot express it honestly |
| T5 | lexical ownership | attaches to one named Node owner and has a closed composition rule |

Loop and the planned Saga pass. Timeout, fallback, audit, trace, validation,
mutex, reducer, aggregator, window, join, routing, and cost measurement do not:
they are attributes, ordinary structures, guide patterns, or observation.

## 6. Patterns are not structural categories

These useful names remain ordinary compositions:

| Guide name | Ordinary form |
|---|---|
| Reducer | Operator with private `var` state |
| Aggregator | Reducer with `instance.key` |
| Window | stateful Operator or Graph with finite `lifetime`, optionally keyed |
| Joiner | Operator with a bounded join handler |
| Dispatch | Graph with `instance.key`, or an Operator's checked Dispatch tree |
| Router | Operator with a closed/open match emitting to static outputs |

They grant no hidden state, scheduling, ports, or runtime. A future distinct
pure execution engine may become another Executor form only with its own real
transition contract; it is not introduced as a decorative label.

## 7. Composition rules

### 7.1 Lexical ownership

An Extension belongs to the Node whose declaration carries it. A nested Node
may own another Extension; the two scopes nest structurally and never
interleave through open/close markers.

### 7.2 State

Manager counters, pending commands, wakeups, and compensation obligations are
folds of the journal. They are not user `var` cells. User mutable state remains
owned by an Operator instance. A binding such as `Loop.state` is read-only.

### 7.3 Spatial and temporal separation

The spatial graph remains a static DAG. Loop recurrence and Saga compensation
are journaled manager transitions, never hidden physical edges. A destination
written as `Loop.next` is compiled as a manager command disposition, not an
edge to a synthetic port.

### 7.4 Instance Closing

Loop commands, delayed recurrence, and run state die with the exact owner
generation. Closing invalidates them and nothing migrates to the successor.
Saga compensation debt is expected to survive through its journal fold, but
that behavior is not executable until Saga's independent decision.

### 7.5 Pairwise composition

Every executable Extension pair needs an explicit composition rule. The
default is never manager order. `App + Loop` means one public Graph whose
finite applications are managed by Loop. `Saga + Loop` remains fail closed
until Saga defines whether a full Loop run is one compensable step and proves
recovery/Closing interactions.

## 8. Names and conflicts

Extension-owned source names are qualified (`Loop.fuel`, `Loop.expired`).
Connector attributes remain governed by their Contract sheet, including the
unchanged `behavior` axis. Business port names are not reserved by Extensions.

Conflict checking occurs before canonical sorting. Two Extensions cannot
contribute incompatible bindings, attributes, commands, ports, admission
rules, or manager ownership to the same Node. A Connector's Contract cannot
redefine an Extension contribution. The diagnostic names both contributors.

## 9. Transport and validation

Plan v17 transports one closed `ExtensionSpec` list on every owner for which it
is non-empty. Registry-free validation re-derives:

- structural owner compatibility;
- canonical ordering and uniqueness;
- every contribution and resolved type;
- conflicts between an Extension and a Connector's Contract;
- runtime-support requirements;
- temporal-manager and journal-fold completeness.

App's presence is explicit on the root owner. The `owners` table remains the
authority for containment, instances, lifetime, Closing, and managers.
`node::Type` is not transported and disappears from the target HIR. A
Connector's exact versioned `ContractId` remains transported in its
specification.

An application handler targets an Operator slot, never a Graph owner. The
independent plan validator rejects a Connector target and an out-of-bounds
slot before a handler can execute. This does not remove controller-owned
Graph lifecycle facts from the journal.

The historical `AspectSpec` tag and `rastok.aspects.r1` domain are never
reinterpreted. V15 and v16 refuse categorically by package version.

## 10. Migration

The words `Aspect`, `NodeAspect`, `Carrier`, and an explicit Graph/Operator
NodeType are removed from the live model. Source using old annotation forms
gets a named migration diagnostic. Historic M1.7 decisions and reports remain
accurate descriptions of v13–v15; D-2026-09-05-1 supersedes their live model.

Old `.`/`.foo` boundary wiring now refuses with `E-WIRING-MIGRATION` and the
exact explicit-boundary replacement. R9 enabled this refusal after the source
twins proved plan/hash neutrality; `out = .` remains a full-envelope value,
not a wiring alias.

Rust target renames are `node::Aspect -> node::Extension`,
`AspectSpec -> ExtensionSpec`, `ConnectorId -> ContractId`, and
`TransactionExecutor -> HandlerExecutor`. The latter frees “Executor” for the
semantic category without asserting a trait. `ConnectorId -> ContractId`
changes no encoded id text or contract identity; it removes only the obsolete
suggestion that the id names a Node rather than its Contract.

## 11. Deferred questions

- user-defined or parameterized Extensions;
- dynamic loading of new Extension semantics;
- Saga's complete protocol and `Saga + Loop` composition;
- Loop on Event, Session, inbound Call, or resident service;
- author-declared Connector variables through a versioned Contract;
- public Rust polymorphism if a real second Executor implementation needs it.

## 12. Related specifications

- [grammar.md](grammar.md) — declarations and annotations;
- [execution-model.md](execution-model.md) — Executor transactions and
  Dispatch;
- [instances.md](instances.md) — addressing, lifetime, lifecycle, Closing;
- [loop.md](loop.md) — Loop manager, commands, proof, and recovery;
- [effects-and-sagas.md](effects-and-sagas.md) — effects and future Saga;
- [delivery-semantics.md](delivery-semantics.md) — routes, journal, boundary,
  recovery.
