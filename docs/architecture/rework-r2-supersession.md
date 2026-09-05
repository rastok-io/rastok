# R2 — v16 normative supersession and review stop

> **Status (2026-09-05): R2 frozen for review.** R0, R1, and R2 are
> specification and architecture work only. The executable workspace remains
> categorical plan v15. R3 is the first source/HIR implementation gate and has
> not started.

The R2 review subsequently accepted the six points in §9, with the three
documentary corrections and exclusive `nothing` recorded below. This document
retains its review-time status; ongoing implementation is recorded in
[the R3 checkpoint](rework-r3-source-hir.md).

## 1. Result protected

The live corpus now describes one model without silently claiming that the
current binary already implements it:

```text
Node
├── Graph
└── Executor
    ├── Operator
    └── Connector ── owns exactly one versioned Contract

Node ── owns zero or more canonical Extensions
```

`Contract` is not a sixth Node category. It is the semantic object owned by a
Connector and identified by `ContractId`. A Driver is a physical
implementation bound to that Contract. `Extension` is the orthogonal protocol
attached to a Node; App and Loop are the closed v16 set, with Saga still a
named future gate.

This separation protects the central invariant: a Graph is passive structure,
an Executor performs atomic application transitions, and a temporal Extension
advances through journal-derived manager commands outside the spatial DAG.

## 2. R0 and R1 evidence

R0 froze four review artifacts:

- [the execution and routing algebra](rework-r0-algebra.md);
- [three hand-written Loop r2 traces](rework-r0-reference-traces.md), for an
  Operator, a Graph, and an outbound Call Connector;
- [the repository-contact audit](rework-r0-repository-audit.md);
- [the pre-allocation identity ledger](rework-r0-identity.md).

R1 froze [the exact host and Contract profiles](rework-r1-host-contracts.md):
entry selection, application boundaries, frame cardinality, terminal facts,
productibility, effect visibility, repetition safety, and quiescence for all
three Loop hosts. It also enumerates the six Contract moves to `@2` without
allocating plan tags or journal keys before R4.

## 3. Live normative homes realigned in R2

| Home | v16 authority after R2 |
|---|---|
| [decisions.md](../spec/decisions.md) | D-2026-09-05-1 and its clause-by-clause supersession ledger |
| [extensions.md](../spec/extensions.md) | Node / Graph / Executor / Connector / Extension; each Connector owns one Contract |
| [grammar.md](../spec/grammar.md) | one Operator handler per trigger, recursive Dispatch, route matches and sets, manager-command syntax, no Graph/Connector application handlers |
| [execution-model.md](../spec/execution-model.md) | shared Executor transaction mechanics, distinct lifecycle controller and temporal managers |
| [delivery-semantics.md](../spec/delivery-semantics.md) | explicit dispositions, source-only observations, Driver ordering, command production/consumption and recovery authority |
| [loop.md](../spec/loop.md) | Loop r2 entry, state, commands, terminal resolution, fuel, delay, proof, quiescence and recovery |
| [instances.md](../spec/instances.md) | passive Graph lifecycle, Executor state/mailboxes and inner-first window closure through child Operators |
| [identity.md](../spec/identity.md) | conceptual v16 movement/non-movement ledger; v15 numeric registry retained until R4 |
| [stdlib.md](../spec/stdlib.md) | current `@1` truth plus exact target `@2` application/cardinality/terminal/effect profiles |
| [effects-and-sagas.md](../spec/effects-and-sagas.md) | effect safety retained; old Connector/Saga handlers removed; future compensation direction only |
| [simulation-semantics.md](../spec/simulation-semantics.md) | App-boundary expectations replace RootSink; mock `on` clauses are Lab Driver scripts, not Connector handlers |
| [envelope.md](../spec/envelope.md) | exact opening admission and LoopContext routing metadata without changing envelope data or seal |
| [diagnostics.md](../spec/diagnostics.md) | named v16 migration, routing, Dispatch, productibility and Loop refusals |
| [WHITEPAPER.md](../../WHITEPAPER.md) | conceptual narrative uses the same five-part ontology and marks R3 as the first executable gate |

[README.md](../../README.md), [ROADMAP.md](../../ROADMAP.md),
[ARCHITECTURE.md](../../ARCHITECTURE.md), the architecture component map and
the Connector Driver contribution guide now use the same target/current
distinction.

## 4. Supersession boundary

D-2026-09-05-1 contains the authoritative table. In summary, v16 supersedes:

- `Kind × Type × Aspect`, Graph application handlers, `ChildPort`, and the
  synthetic `__graph_handler` checking context;
- port-shaped Loop recurrence and the r1 run recipe based only on root
  lineage;
- implicit RootSink/Unwired output behavior and port-name special cases;
- linear-only handler bodies and deferred multi-emission;
- UI projections which invent Graph-handler or old Loop control ports.

It retains owner/address authority, instances, lifetime, Closing, joins and
their admission `where`, typed envelopes, Driver roles and exchanges, the
three journal record kinds, logical time, data annotations, and the existing
effect classification.

The M1.7 backlog and exit report remain accurate evidence for the v13/v14
implementation which actually shipped. M1.3–M1.8 backlogs, exit reports and
the older decision entries are historical records, not competing live
semantics. They are not rewritten to pretend the superseded model never
existed.

## 5. Corrections fixed before R3

Repository contact and R2 review found six places where the opening prose
needed precision:

1. A Graph owns no **application** transaction; generic lifecycle
   `l:start`/`l:closing`/`l:stop` records remain attributed to Graph owners.
2. Only world-originated Driver facts become INBOUND roots. A result correlated
   to an open outbound exchange settles that obligation and activates the
   Connector without inventing a second root.
3. Recoverable Loop commands require distinct durable `CommandProduced` and
   `CommandConsumed` facts. A RAM queue or one conflated transition cannot
   recover the crash window.
4. A proof-only Loop can make `Loop.expired` impossible. An explicit smaller
   fuel cap can still make it productible unless the proof also establishes
   termination within that cap.
5. `origin_id` cannot distinguish two same-root deliveries to one instance.
   `LoopRunRef` therefore includes exact `OpeningAdmissionRef`, and routed
   work carries a journaled LoopContext stack.
6. `nothing` is an exclusive disposition for one source case, including
   across separate declarations. Otherwise its meaning would depend on a
   distant additive route: `source -> nothing` could silently mean “discard”
   or “also deliver elsewhere.” Auditing a routed union is written inside
   each applicable match arm rather than as a universal route beside a
   `nothing` arm.

These are amendments to the candidate wording, not implementation shortcuts.

## 6. Current versus target truth

| Concern | Executable at the R2 stop | Normative v16 target |
|---|---|---|
| plan package | v15 | one categorical v16 bump at R4 |
| Node model | current Kind/Type/Aspect implementation | Node / Graph / Executor / Connector / Extension |
| Contract id Rust name | `ConnectorId` | `ContractId`; encoded id text and meaning unchanged |
| Graph handlers | executable historical v15 path | forbidden; removed at R3–R7 |
| outputs | current `UnwiredPolicy`/RootSink behavior | productibility plus explicit disposition at R5 |
| handler body | current guard plus linear steps | recursive Dispatch at R3/R4/R6 |
| route match and sets | parsed/deferred or absent | checked and executable at R3/R4/R6 |
| multi-emission | parsed and fail-closed | ordered atomic emission at R3/R6 |
| Loop | v14/v15 Graph-handler r1 | r2 over Operator, Graph and outbound finite Call at R7/R8 |
| recovery | spatial Loop body refuses | complete spatial RecoveryFold at R7 |
| UI | retained M1.9 work, paused | rebuilt from v16 semantics only at R9 |

New v16 syntax remains a named refusal until its implementation gate. No R2
document is evidence that current binaries execute it.

## 7. Deliberately unmigrated material

R9 owns the global fixture migration. Until then, examples, benchmark prompts,
goldens, tests, M1.7 evidence, and the paused UI may still contain old boundary
sugar, Graph handlers, port-shaped Loop control, or historical taxonomy. They
must not be used as v16 normative authority. Keeping them unchanged at R2 is
also what makes the later neutral-sugar and scoped identity tests possible.

No executable source file, test fixture, package golden, semantic hash, causal
trace, or protocol dependency is changed by R0–R2.

## 8. R2 exit checklist

- [x] The ontology has exactly Node, Graph, Executor, Connector and Extension;
      every Connector owns one Contract.
- [x] Operator and Connector share mailbox/transaction/semantic-state/ordered-
      emission mechanics while retaining different transition provenance.
- [x] Graph lifecycle records are distinguished from Graph application code.
- [x] RouteRule and PhysicalEdge have separate meanings.
- [x] Destination sets and multi-emission have distinct envelope identities.
- [x] One-port `where` identity-neutral desugaring and join `where` admission
      semantics are both explicit.
- [x] Productibility and explicit dispositions replace implicit sinks.
- [x] Loop commands are durable manager inputs, not ports or DAG edges.
- [x] `Loop.expired` is the sole Loop-contributed output and follows the common
      disposition rule.
- [x] Operator, Graph and outbound Call state machines and recovery traces are
      written before implementation.
- [x] Contract `@2` movements and v16 non-movements are enumerated before any
      re-bless.
- [x] The Whitepaper and live specification use the same model.
- [x] R3 has not started.

## 9. Review gate before R3

R3 may open only after review accepts:

1. the exact opening-admission and LoopContext model;
2. Produced-versus-Consumed command durability;
3. natural Connector Call Done versus an explicitly routed recurrence;
4. conservative `@2` effect-visibility tables;
5. productibility as re-derived proof, never a trusted serialized boolean;
6. the single categorical v16 movement ledger.

Until then, the implementation remains untouched and this document is the
handoff point.
