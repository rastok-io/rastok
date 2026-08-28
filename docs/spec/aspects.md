# Node kinds, types, and aspects

> **Status:** normative taxonomy. `node::Kind` and `node::Type` are executable.
> `Loop` and `Saga` are recognized `node::Aspect` values but remain named
> execution gates until their individual specifications land.

This document defines the common classification model used by Graphs,
Operators, and Connectors. The aspect-specific protocols live in
[loop.md](loop.md) and [effects-and-sagas.md](effects-and-sagas.md).

## 1. Three orthogonal facts

Every Node has exactly one kind, exactly one effective type, and zero or more
aspects:

| Fact | Question answered | Cardinality | Examples |
|---|---|---:|---|
| `node::Kind` | Which structural declaration family is this? | exactly 1 | `Graph`, `Operator`, `Connector` |
| `node::Type` | Which implementation or contract does this Node have? | exactly 1 | `App`, `Graph`, `Operator`, `HttpClient`, `Process` |
| `node::Aspect` | Which additional orchestration protocol wraps it? | 0..n | `Loop`, `Saga` |

The declaration keyword fixes the kind. The annotation can select the type and
add aspects, but it cannot change the kind:

```rastok
graph app : App { ... }                    // Graph / App / []
graph refinement : Loop { ... }            // Graph / Graph / [Loop]
operator normalize { ... }                 // Operator / Operator / []
connector api : HttpClient { ... }         // Connector / HttpClient / []
```

A type says what the Node is. An aspect says which separately specified
protocol alters its temporal orchestration. Neither is a “role”. In
particular, `App` is the unique deployed-root Graph type.

## 2. Source shape and defaults

The single annotation production is:

```ebnf
node_annotation ::= IDENT ( "+" IDENT )*
                  // optional NodeType first, then NodeAspects
```

The checker classifies symbols against the type and aspect registries. The
rules are closed:

1. If a NodeType is written, it is the first symbol.
2. An ordinary Graph's default type is `Graph`.
3. An ordinary Operator's default type is `Operator`.
4. A Connector has no default: its exact contract NodeType is mandatory.
5. Omission and the explicit default are one program. For example,
   `operator x {}` and `operator x : Operator {}` have the same HIR identity,
   plan, and semantic hash.
6. Aspects are canonicalized by their registered order. Source order between
   distinct compatible aspects carries no identity.
7. A duplicate, unknown, misordered, or incompatible symbol is
   `E-NODE-ANNOTATION`.
8. A known aspect whose runtime gate has not landed is a named
   `E-CHECK-DEFERRED`; it never reaches an executable plan by accident.

The kinds admit these effective types:

| Declaration kind | Effective NodeType |
|---|---|
| `graph` | implicit `Graph`, explicit `Graph`, or the unique root `App` |
| `operator` | implicit/explicit `Operator`, or a registered pure implementation such as a future `Jaq`/`Wasm` |
| `connector` | one exact registered Connector contract type such as `Process` or `HttpClient` |

Writing a Connector type on a Graph, `App` on an Operator, or a second type
after an aspect is not reinterpretation territory; it refuses.

## 3. Patterns are not types

The following names describe useful configurations of ordinary primitives.
They do not select another runtime or grant hidden behavior:

| Guide name | Ordinary form |
|---|---|
| Reducer | Operator with Node-local `var` state |
| Aggregator | Reducer with `instance.key` |
| Window | stateful Operator or Graph with a finite `lifetime`, optionally keyed |
| Joiner | Operator with a bounded `on (a, b, …)` handler |
| Dispatch | Graph with `instance.key` |
| Router | Operator performing conditional emission to statically declared output ports |

The compiler refuses these names when written as NodeTypes. This keeps the
program honest: state, addressing, lifetime, join bounds, ports, and emissions
remain visible in source and in review. A future genuinely distinct pure
engine is allowed to become an Operator NodeType because it has a real
implementation contract rather than a decorative alias.

`Router` remains a guide-level name until the generic conditional
multi-output statement is specified and executable. It must not return as a
privileged family merely to hide that missing surface.

## 4. Initial aspect compatibility

The initial closed aspect vocabulary is:

| Aspect | Compatible carriers | Owns |
|---|---|---|
| `Loop` | Graph in M1.7 | finite temporal recursion, `next`/`expired`, fuel, `l:next`, optional delay |
| `Saga` | Graph only | journal-folded compensation over effects inside the lexical Graph |

M1.7 deliberately gives Loop one carrier and one proof boundary. Pure
Operator work and outbound Connector calls participate as children of a Loop
Graph. An inbound Event source, accepted Call or Session cannot be retried as
an operation merely because its Driver also writes responses. Direct
`Operator + Loop` and `ConnectorType + Loop` remain named later gates rather
than acquiring carrier-specific continuation rules.

A Connector inside a Saga is not annotated `+ Saga`. Its `effect` contract
and lexical containment make it a saga step:

```rastok
graph checkout : Saga {
  graph reserve_with_retry : Loop { ... }
  graph charge_with_retry  : Loop { ... }
  . -> reserve_with_retry -> charge_with_retry -> .
}
```

## 5. Aspect admission test

Aspects are intentionally scarce. A proposed aspect is admitted only if all
five tests pass in writing:

| # | Test | Required answer |
|---:|---|---|
| T1 | Runtime layer | It needs a real state machine/scheduler beyond an existing Node, instance, Connector, or aspect protocol. |
| T2 | Dedicated vocabulary | It owns clauses, attributes, hooks, or typed ports that are meaningless outside its scope. |
| T3 | Active orchestration | It changes envelope flow, timing, retry, or compensation rather than merely observing it. |
| T4 | Non-redundancy | Existing ports, handlers, state, `instance.key`, lifetime, and wiring cannot express it naturally. |
| T5 | Lexical scope | It wraps a Node body or Graph region rather than parameterizing one ordinary operation. |

`Loop` and `Saga` pass. Timeout, fallback, audit, trace, validation, mutex,
reducer, aggregator, window, join, and routing do not: they are existing
attributes/primitives, runtime-wide observation, or guide patterns.

## 6. Structural rules shared by every aspect

### 6.1 Vocabulary is annotation-gated

Aspect-owned names are legal only in a Node carrying that aspect:

```rastok
graph retry : Loop {
  Loop.fuel = 5
}
```

Omitting `: Loop` is an error; it never enables iteration through duck typing.
Conversely, writing `: Loop` while the Loop transport/runtime gate is absent is
an explicit deferral, never ordinary execution.

### 6.2 The spatial graph remains a DAG

Loop re-entry (`l:next`) and Saga compensation (`l:compensate`) are dated,
journaled temporal facts. They are not hidden graph edges. The declared
topology remains acyclic and statically enumerable.

### 6.3 Scope is lexical

An aspect's scope is the braces of the declaration carrying it. Nested
aspects compose by proper lexical nesting. No open/close markers or named
cross-graph binding may create an interleaved scope.

### 6.4 Aspect state is not a user `var`

Runtime-owned attempt counters, pending wakeups, or compensation obligations
belong to the aspect's journal fold. User state remains Node-local `var`
state. The aspect specification may expose read-only contextual values, but
user code cannot mutate the fold directly.

### 6.5 Contributed ports remain typed ports

An aspect may contribute typed ports. After contract resolution they obey the
same `DataType`, emission, journal and unwired-policy rules as every other
port. Loop contributes `next: In` and
`expired: LoopExpired { last_state: In }`. `next` is its one deliberate
control-port exception: it is emitted by ordinary handlers but only the Loop
manager may consume it, so source wiring to it refuses. Incompatible
name/type collisions refuse at check time. Saga's eventual ports are specified
by Saga rather than generalized here.

### 6.6 Lifecycle is distinct from Node-instance lifecycle

Bare `on start` / `on stop` address the current Node instance. Aspect protocols
do not acquire hooks automatically. Loop has no lifecycle hooks: its run opens
from a root `in` envelope and closes on `out` or `expired`. Instance Closing
still governs the enclosing mailbox and open Connector obligations; each
aspect specification states which of its obligations die or survive there.

For the initial aspects:

| Obligation | Scope at instance Closing |
|---|---|
| pending Loop `l:next` / delay | dies with that instance generation |
| Saga compensation debt | survives as a journal fold and is re-derived during recovery |

## 7. Composition and conflicts

Each aspect pair needs an explicit composition rule before the pair is
executable. The default is not “whichever manager runs first”. The target
`Saga + Loop` rule makes one complete finite Loop run resolve inside one saga
step; only the terminal result becomes the step's durable outcome. The pair
remains fail-closed until Saga executes and the rule is tested.

Aspect-owned attributes use their qualified owner (`Loop.fuel`,
`Saga.timeout`). Connector attributes remain governed by the Connector sheet.
There is no priority rule in which a type silently wins over an aspect or one
aspect wins over another. Any unqualified collision refuses.

## 8. Transport and identity

The current v12 plan has no executable aspects. It derives all current
NodeTypes from existing categorical facts and does not serialize a duplicate
claim:

- parentless Graph owner → `App`;
- other Graph owner → `Graph`;
- Operator plan entry → `Operator`;
- Connector spec → `Connector(id)`.

The first executable aspect will add one closed aspect list at the plan gate,
with Registry-free validation and an enumerated semantic/package identity
move. A runtime never rediscovers aspects from source strings, attributes, or
port names.

## 9. Deferred questions

- User-defined and parameterized aspects are outside the current contract.
- Every future pair requires a composition matrix entry.
- M1.7 must close the Graph boundary decision proof, run identity, fuel,
  typed exhaustion and source-to-Lab evidence before removing the current
  `E-CHECK-DEFERRED`.
- Saga remains later than Loop; this document fixes its classification but
  does not claim an executable manager.

## 10. Related specifications

- [grammar.md](grammar.md) — declarations and annotation production.
- [instances.md](instances.md) — orthogonal addressing, lifetime, and Closing.
- [loop.md](loop.md) — bounded re-application protocol.
- [effects-and-sagas.md](effects-and-sagas.md) — effects and Saga fold.
- [delivery-semantics.md](delivery-semantics.md) — journal-before-effect and
  close-once obligations.
