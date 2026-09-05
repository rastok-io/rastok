# R0 — Node, Graph, Executor, Connector, Extension, routing, Dispatch, and Loop algebra

> **Status (2026-09-05): R0 frozen for review.** This document is the
> implementation algebra behind D-2026-09-05-1. The live normative homes are
> the documents under `docs/spec/`; this file records the model that R1 and R2
> were checked against. It does not claim that plan v16 or its runtime exists.

## 1. Result protected

The rework protects one central invariant:

> A Graph describes structure; an Executor performs atomic transitions; an
> Extension changes a Node's contract; a temporal Extension is advanced by a
> journal-derived manager that is outside the spatial DAG.

That separation removes three accidental execution paths from v15:

- a Graph handler compiled through a synthetic Operator;
- a child-output observation path alongside ordinary routed delivery;
- an implicit root sink which gives an unwired output runtime meaning.

The replacement does not weaken the existing transaction rule. For every
Executor or temporal manager transition, consumed positions, semantic state,
ordered products, selected destinations, and dated obligations either commit
together or do not commit.

## 2. Closed ontology

```text
Node
├── Graph
│   └── typed boundary, children, wiring, lexical vals,
│       instance/lifetime/Closing, Extensions
└── Executor
    ├── Operator
    │   └── Dispatch DSL, private vars
    └── Connector
        └── Contract, typed Driver events, boundary state

Extension(owner: Node)
├── App                 static; root Graph only
├── Loop                temporal; admissible finite applications only
└── Saga                temporal; future gate

Temporal Extension ── owns ──> Manager
                                bounded command inbox
                                journal-derived state
                                no Node, port, or PhysicalEdge identity
```

The categories are disjoint:

`Contract` is not another Node category. Every Connector owns exactly one
versioned Contract, identified by `ContractId`; Drivers are physical
implementations bound to that Contract.

| Capability | Graph | Operator | Connector | temporal manager |
|---|---:|---:|---:|---:|
| typed ports | boundary only | yes | yes | no |
| logical instance/lifetime/Closing | yes | yes | yes | owner identity |
| ordinary spatial mailbox | no | yes | yes | no |
| bounded activation/command inbox | no | delivery activations | delivery and Driver activations | commands |
| application transaction | no | yes | yes | yes |
| semantic state in the journal | no | DSL `var` | Contract state, exchanges, cursors | Extension fold |
| DSL `var` | no | yes | no in v16 | no |
| DSL handler | no | yes | no | no |
| Driver | no | no | yes | no |

`Executor` is a semantic category, not a required Rust trait. A trait is added
only if two real implementations need a substitution boundary. The existing
common transaction machinery can remain concrete.

### 2.1 What “a Graph has no transaction” means

The literal sentence is too broad for the existing journal contract. A Graph
instance still has `l:start`, `l:closing`, and `l:stop` facts. They remain
ordinary `TRANSACTION` records attributed to the Graph owner so recovery can
rebuild containment and generations. They are committed by the generic
lifecycle controller, not by Graph code.

The precise invariant is therefore:

> A Graph owns no application mailbox, handler, mutable application state, or
> application transaction. The lifecycle controller may commit lifecycle
> transaction records attributed to any Node owner, including a Graph.

This distinction is normative in v16 and is used throughout this document.

### 2.2 The common Executor transition

An Executor transition has the abstract form:

```text
transition :
  (ExecutorSpec, InstanceRef, Activation, SemanticState, Context)
    -> PreparedCommit | Fault

PreparedCommit = {
  consumed_positions,
  exchange_transition?,
  state_delta,
  ordered_emissions,
  produced_manager_commands,
  selected_routes,
  armed_timers,
  close_request?
}
```

Preparation is pure with respect to the journal and Drivers. It resolves types,
instances, route cases, manager commands, and every capacity. A successful
journal commit makes all prepared products visible. Only then may an OUTBOUND
command reach a Driver.

Operator and Connector differ only in how `Activation -> PreparedCommit` is
described:

- an Operator uses its validated `Dispatch`;
- a Connector uses its versioned Contract and a typed Driver event.

A Connector's semantic state belongs to the core and is journaled. Socket,
process, pool, file handle, and protocol-library state belong to its Driver and
are reconstructible physical state. A Driver cannot mutate the semantic state
or journal directly.

## 3. Activations and Driver facts

An Executor inbox is bounded. It may contain two kinds of activation:

```text
Activation = SpatialDelivery(DeliveryRef, Envelope, Port)
           | CorrelatedDriverEvent(ExchangeRef, TypedOutcome)
           | Lifecycle(LifecycleKind, Cause)       // controller-owned
```

The first two can drive application transactions. Lifecycle is a separate
controller transition and may invoke an Operator's `on start` or `on stop`.

Not every Driver event is an `INBOUND` root:

- a world-originated Event, inbound Call, Session open, or Session message is
  admitted as an `INBOUND` fact before any Connector transaction consumes it;
- an outcome correlated to an already-open outbound Call or Session write is
  not a new root. It is retained in the bounded Connector activation path,
  closes or settles the existing OUTBOUND obligation, and supplies the typed
  Connector transaction.

Inventing an INBOUND record for the second case would create a false root and
break the existing Call/Session causal taxonomy. Plan v16 preserves this
distinction.

## 4. Ports, endpoints, and dispositions

A `Port` is a typed interface endpoint owned by a Node. Extension commands are
not ports. The plan distinguishes the following concepts:

```text
PortEndpoint       = (NodeOwner, Port, Direction, DataType)
PhysicalEdge       = (source PortEndpoint, destination PortEndpoint, Capacity)
PublicTermination  = App boundary output
ManagerDestination = (ExtensionOwner, ManagerCommandKind)
Discard            = nothing

Disposition = PhysicalEdge
            | PublicTermination
            | ManagerDestination
            | Discard
```

Direction is relative to the Graph body:

- the current Graph's input boundary is a source inside the body;
- the current Graph's output boundary is a destination inside the body;
- a child input is a destination;
- a child output is a source.

A passive Graph boundary is not a mailbox. Resolution may flatten its ordinary
spatial seam, but must preserve the boundary crossing, owner, instance, data
policy, and any Extension admission gate.

### 4.1 Physical edges and route rules

`PhysicalEdge` answers where an ordinary delivery can travel. `RouteRule`
answers which dispositions apply to a produced value:

```text
RouteRule = {
  source: PortEndpoint,
  by_case: ClosedCaseMap<CaseTag, CanonicalDispositionSet>
}
```

For an untagged value, the table has one universal row. A simple route adds its
destination to every row. A route `match` adds destinations only to its selected
rows. `nothing` is exclusive: it cannot coexist with a real or manager
destination for the same source case, even when the conflict comes from two
separate declarations.

The case is not part of `PhysicalEdge` identity. One source/destination pair is
one physical edge even when several cases select it. The validated plan stores
the closed case table separately and re-derives it from source type and rules.

Destination sets are mathematical sets. Their source order is non-semantic;
lowering sorts them canonically. Source sets expand to independent mpsc edges
and are also canonically ordered. A path chain remains ordered syntax only
because it expands into adjacent physical edges; it is never one fused
transaction.

### 4.2 One envelope versus several envelopes

These forms deliberately differ:

```rastok
source -> { left right }     // one envelope, two DeliveryRefs
(left, right) = value        // two envelopes, two EnvelopeIds
```

The first route reserves both destinations atomically. Its target set is
canonical and unordered. The second is an ordered multi-emission and is covered
by §7.

## 5. Productibility and explicit destinations

Each output and each closed case has a re-derived productivity:

```text
Productivity = Impossible | Possible
```

`Impossible` requires proof from validated material. Absence of evidence is
`Possible`. V16 admits only these proof sources:

1. **syntactic:** no reachable `Dispatch` leaf emits the Operator output;
2. **structural:** no productive source reaches a Graph boundary output;
3. **typed:** a closed narrowing makes the case empty;
4. **contractual:** the resolved Connector profile declares the port/case
   cardinality `never`;
5. **reachability:** an entire Graph is unreachable, in which case its local
   obligations are suspended under `W-DEAD-GRAPH`.

Every proof is re-derived by the core validator. A serialized boolean or
compiler claim is insufficient. The first v16 implementation has no general
theorem prover; anything outside this fragment remains `Possible`.

The obligation is:

```text
Possible(source case) => selected dispositions are non-empty
```

Failure is `E-UNWIRED-OUTPUT`, naming the owner, port, and case when applicable.
An impossible declared output receives `W-DEAD-OUTPUT` and needs no artificial
`-> nothing`. An input with no possible producer receives `W-DEAD-INPUT`; the
required response half of an inbound Call in `respond` mode remains an error.

If a Driver produces a port or case whose contractual productivity is
`Impossible`, the core raises `ContractViolation` before publishing or
discarding that value.

## 6. Dispatch algebra

Only Operators own a `Dispatch`:

```text
Dispatch = Linear([Step])
         | ClosedMatch {
             scrutinee: Expr,
             arms: CanonicalMap<NonEmptyCaseSet, Dispatch>
           }
         | OpenMatch {
             arms: [(Predicate, Dispatch)],   // source order
             fallback: Dispatch
           }
```

Closed arms are disjoint and exhaustive over a tagged union, enum, or bool.
They are sorted by canonical tags; source arm order is not semantic. Open arms
are evaluated in source order; first true wins and a fallback is mandatory.
Only one leaf executes.

`Linear` is rectilinear. It contains pure local values, assignments,
multi-emissions, `stop`, `nothing`, and manager-command statements. A nested
`Dispatch` may occur only at the dispatch position of a block or arm. No state
mutation, emission, command, or stop may precede it; pure `val` declarations
may.

### 6.1 The two meanings of `where`

On one input port, `where` is source sugar for an open match:

```rastok
on in where P { body }
```

lowers before semantic identity to:

```rastok
on in match {
  P => { body }
  _ => nothing
}
```

A false predicate therefore consumes the input and commits the empty
transaction. Both spellings produce the same HIR `Dispatch`, plan bytes, and
semantic hash. `where P match { ... }` becomes an outer open match whose true
arm is the inner dispatch.

On a join, `where` is not Dispatch. It is `admission_guard` on the join
activation, evaluated before any candidate is consumed. False retains every
candidate and commits nothing. The eventual Dispatch sees the consumed tuple.

There is at most one handler and one `where` per activation. Multiple business
branches belong in one `match`.

### 6.2 Identity of selected arms

An arm identifier is unnecessary. Exactly one arm runs for a given transaction,
and emission identity already includes owner, trigger, causal parent, port, and
per-port emission index. Closed-arm canonical order and open-arm source order
are plan identity, while a non-selected arm creates no runtime identity.

The validator independently checks maximum depth, closed-domain suitability,
disjointness, exhaustivity, canonical order, and open fallback presence.

## 7. Ordered emissions and atomic multi-emission

Statements are evaluated in source order. That order determines state reads,
emission journal order, and each per-port `emission_index`. The entire selected
Dispatch leaf is one atomic transaction.

Three forms are distinct:

```rastok
left = first(data)
right = second(data)
```

evaluates two right-hand sides in order and creates two envelopes in that order.

```rastok
(left, right) = value
```

evaluates `value` once, then creates one envelope per target in written order.
Each envelope has its own `EnvelopeId`, the same cause and `origin_id`, and the
same data value. Repeating a target is `E-MULTI-EMIT-DUPLICATE`.

```rastok
(left, right) = (first(data), second(data))
```

requires equal arity, evaluates values left-to-right, then creates the paired
emissions in target order.

For every form, the core selects all route cases and target instances and
reserves every ordinary mailbox and manager command slot before commit. A
failure leaves state, cursors, commands, emissions, and journal unchanged.

Unlike a destination set, multi-emission target order is semantic. Swapping
`left` and `right` changes the ordered products and may change their ids.

## 8. Extensions and managers

An Extension is a canonical, closed specification attached to one Node owner.
It may contribute attributes, bindings, admission constraints, commands, and
ports. It need not contribute every category. Extension source order is erased
after conflict checking.

`App` contributes deployment-root meaning and a public boundary, but no
manager. `Loop` contributes three commands and one possible output. `Saga` is a
future independent decision.

A temporal manager is runtime machinery identified by `(owner InstanceRef,
Extension kind)`. It is not a Node, has no port, and never appears in the
spatial endpoint table. Its one-slot-per-run command inbox plus a global bounded
work cap provide backpressure. Its state is a fold of journal facts; RAM queues
are disposable indexes.

### 8.1 Command production and consumption are separate facts

A manager command must survive a crash between its producer commit and manager
consumption. V16 therefore records both stages:

```text
ManagerCommand = {
  ref: ManagerCommandRef,
  target: (Loop owner InstanceRef, LoopRunRef),
  iteration: u64,
  kind: Next | Again | Done,
  payload: NextEnvelope | CauseId
}

Produced(command)   // part of the producer's atomic transaction
Consumed(command)   // part of the manager's later transition
```

The command inbox is exactly the fold `Produced - Consumed - Invalidated`.
`Produced` is carried in an extend-only transaction field; it is not an
OUTBOUND record, port emission, or PhysicalEdge. A natural terminal output may
produce an implicit `Done` command in the same record.

`ManagerCommandRef` is derived from the target run, iteration, command kind,
causal source id, and a command ordinal. The ordinal is normally zero because a
Loop application admits exactly one verdict; retaining it makes duplicate
production diagnosable rather than aliasing identities.

## 9. Loop context and opening admissions

The v15 Loop run recipe uses the lineage root. That is insufficient when one
root envelope reaches the same instance twice. V16 introduces an exact opening
admission reference:

```text
OpeningAdmissionRef = RootIngressRef(InboundRef, entry_port)
                    | SpatialDeliveryRef(DeliveryRef)

LoopRunRef = derive(
  "rastok.loop-run.r2",
  InstanceRef,
  OpeningAdmissionRef
)
```

The target instance remains recipe material. Two distinct spatial deliveries
of one envelope cannot alias, even if they retain the same `origin_id`.

### 9.1 Causal Loop context

Origin lineage alone cannot identify a run after fan-out. Each routed delivery
therefore carries a journaled `LoopContext`, a stack of active `LoopRunRef`
values from outermost to innermost. It is routing metadata, not `meta.sys`, user
data, or a second identity authority.

- opening a Loop host pushes its run;
- internal routes inherit the stack;
- leaving a Loop scope pops that run;
- a nested Loop pushes another run and masks `Loop.state`;
- a transaction inherits the exact context of its consumed delivery;
- all members of a join inside a Loop application must have the same context.

A statically visible cross-context join is `E-LOOP-CONTEXT`. A forged or
runtime-mixed candidate is `LoopContextMismatch` and consumes nothing. This
rule is required for `Loop.state`, quiescence, commands, and recovery to name
one run without guessing from `origin_id`.

The context is stored on routed-admission facts and re-derived along consumed
positions. It does not alter the envelope seal. Its exact v16 encoding is
listed in the identity ledger.

## 10. Loop interface resolution

Loop reuses its host's ordinary interface and adds only what is necessary:

| Contribution | Kind | Rule |
|---|---|---|
| `Loop.fuel` | attribute | positive application cap |
| `Loop.delay` | attribute | optional fixed/exponential recurrence delay |
| `Loop.measure` | attribute | checked termination proof |
| `Loop.entry` | attribute | selects recurrent input when convention cannot |
| `Loop.terminal` | attribute | selects ordinary outputs which imply Done |
| `Loop.state` | binding | complete current entry envelope |
| `Loop.next` | command | supplied envelope becomes next state |
| `Loop.again` | command | retain current state; source is only cause |
| `Loop.done` | command | close run without a business envelope |
| `Loop.expired` | output | manager-produced `LoopExpired<entry type>` |

Commands are written like destinations for wiring ergonomics but are not ports.
They never occur as a source, never get a PhysicalEdge, and never appear in a
Node port table.

### 10.1 Entry

The recurrent entry is resolved in this order:

1. the host has exactly one input;
2. otherwise exactly one input is named `in`;
3. otherwise `Loop.entry = name` is required.

Ambiguity is `E-LOOP-ENTRY`. The resolved port and exact opening admission are
transported. Every recurrence re-enters that same port; there is no `next`
port or back-edge.

### 10.2 Terminal outputs

For an Operator or Graph:

- one ordinary output is terminal by default;
- several ordinary outputs require `Loop.terminal = [a, b, ...]`;
- no ordinary output requires explicit `Loop.done` on every natural terminal
  path.

The list is canonical and names only owner outputs. Emitting one or more
terminal outputs in one Executor transaction produces one implicit Done
command. A later transaction in the same Graph application cannot produce a
second terminal verdict while the first is pending.

For an outbound Connector Call, frames (`out`, `err`) are non-terminal. The
single terminal `exit` closes the application. A route selected from an exit
case may produce `Again`, `Next`, or explicit `Done`; otherwise the exit
produces natural Done. If an earlier frame already produced the application's
command, the exit closes the Call but does not invent a second Done.

## 11. Loop manager state machine

### 11.1 States

```text
Dormant
  └─ opening admission retained
       -> Opening(run, entry, state, budget)
            -> Running(run, iteration=0, remaining=budget)

Running
  ├─ Executor/contained work active
  ├─ command Produced but obligations open -> VerdictPending
  ├─ quiescent with no command             -> LoopNoVerdict
  └─ second incompatible verdict           -> LoopProtocolViolation

VerdictPending + quiescent
  ├─ Done    -> Closed(Natural | ExplicitDone)
  ├─ Stopped -> Closed(Stopped) + instance Closing
  ├─ Next/Again, remaining > 1
  │    -> Waiting(due?) -> Admitting -> Running(iteration+1, remaining-1)
  └─ Next/Again, remaining == 1
       -> Expired(last_state) -> Closed(Expired)
```

An opening route is an ordinary retained admission annotated with its
`OpeningAdmissionRef`. The manager records `Open` before the host may consume
the admission. `Admit` releases the initial delivery, or on recurrence creates
the new delivery/routes only after capacity has been proved.

### 11.2 Fold facts

V16's Loop r2 fold distinguishes retained input, command production, and
manager consumption:

```text
Open {
  run, instance, opening_admission, entry_port,
  initial_budget, state0
}

CommandProduced {
  command_ref, run, iteration, kind, payload_or_cause
}                                      // producer transaction

CommandConsumed {
  command_ref, run, iteration
}

Advance {
  run, from_iteration, to_iteration, remaining,
  command_ref, next_state, due_at_annotation?
}

Admit {
  run, iteration, admission_ref
}

Close {
  run, iteration, Natural | ExplicitDone | Stopped
}

Expired {
  run, iteration, command_ref, last_state, output
}
```

These are extensions of INBOUND or TRANSACTION facts, never a fourth record
kind. `CommandProduced` may occur in an Operator/Connector transaction which
also emits ordinary envelopes. Manager transitions use TRANSACTION records
attributed to the Extension manager's owner. `due_at` remains a scheduler
annotation; the timer reference and eventual transition remain causal.

Exactly one unconsumed verdict command may exist per `(run, iteration)`. The
global manager-work cap bounds the number of live runs and pending manager
actions.

### 11.3 Fuel

`Loop.fuel = N` counts applications, including the first. The fold is:

```text
run(0, state) = Expired(state) before application

run(N + 1, state) =
  apply(state), then
    Done       -> Close
    Stopped    -> Close and close instance
    Next(s')   -> run(N, s')
    Again      -> run(N, state)
```

Source admission requires positive fuel, so the practical first state has at
least one application. When the final permitted application requests Next or
Again, the manager does not create `l:next` or reserve recurrence routes. It
emits `Loop.expired` with the proposed Next state or retained Again state.

### 11.4 `Loop.expired` productibility

`Loop.expired` follows the common explicit-destination rule. It is possible
whenever an admitted recurrence can exhaust the effective cap.

For a root `App + Loop` Graph, the contributed output is itself a public App
boundary termination and therefore already has a disposition. Every non-root
Loop owner exposes it only to its parent. A local fallback requires a wrapper
Graph rather than an internal back-route.

A measure-only Loop whose checked certificate proves natural termination within
its derived budget makes `Loop.expired` impossible. When explicit fuel is also
present, fuel remains the deliberate runtime cap; `Loop.expired` stays possible
unless the validator can prove termination no later than that explicit cap. V16
may conservatively keep it possible in this mixed case.

### 11.5 Delay and admission

The manager consumes a Next or Again command only after the application is
quiescent. It computes the saturated fixed/exponential delay and arms timer kind
5. No future destination capacity is held while waiting. At the due logical
instant, `Admit` rechecks run and owner generation, resolves the remembered
entry route, reserves all current capacities, commits, and only then exposes
deliveries.

Closing wins every race. It invalidates unconsumed commands and wakeups for the
generation; no state migrates to a successor.

### 11.6 Quiescence

Quiescence is indexed by `LoopRunRef`, not by origin:

- no internal delivery belonging to the run is pending;
- no Executor transition for the run is active;
- no contained outbound Call or Session write is open;
- no contained application timer or child-manager command is pending;
- the selected verdict command itself is excluded.

Work outside the host scope or belonging to another run does not block.
Ordinary outputs may leave the scope before quiescence; their external
descendants do not become hidden Loop obligations.

## 12. Host-specific application machines

### 12.1 Operator host

```text
Admit(entry delivery)
  -> one Operator Dispatch transaction consumes it
  -> state delta + ordered emissions + exactly one verdict commit atomically
  -> Operator application is complete
  -> manager consumes verdict
```

A statically exhaustive Dispatch proves verdict uniqueness leaf by leaf.
Runtime checks remain for forged plans and values outside contractual proofs.

### 12.2 Graph host

```text
Admit(Graph boundary input)
  -> route into contained Executors
  -> zero or more contained transactions / Calls / timers
  -> one terminal boundary route or explicit manager command is retained
  -> wait for run-scoped contained obligations to quiesce
  -> manager consumes verdict
```

The Graph performs no transaction. The transaction which crosses a terminal
boundary records the implicit Done command for the Graph's manager. A child
output routed to `Loop.next/again/done` records that command in the child's
transaction.

### 12.3 outbound Connector Call host

```text
Admit(request delivery)
  -> Connector transaction seals call intent
  -> OUTBOUND open commits
  -> Driver starts only afterwards
  -> zero or more typed frame events become Connector products
  -> exactly one exit closes the Call and completes the application
  -> selected recurrence command or natural Done
  -> manager consumes verdict after Call closure
```

The terminal Contract classifies effect visibility conservatively. `Again` or
`Next` after `before_effect` is always safe. `outcome_unknown` or `after_effect`
requires `effect = none | idempotent`; `compensable` alone is insufficient.

## 13. Recovery obligation

Recovery folds the journal in this order:

1. owner generations and lifecycle;
2. semantic state deltas and consumed cursors;
3. committed routed deliveries not yet consumed, including LoopContext;
4. OUTBOUND Calls/Sessions and retained correlated Driver outcomes;
5. dated obligations;
6. Loop r2 runs, produced/consumed commands, and due admissions.

A `CommandProduced` without `CommandConsumed` is requeued exactly once. The
producer transaction and any external effect are never replayed. A consumed
Advance with no subsequent due Admit recreates the timer or immediate admit.
An Admit already in the journal is never repeated.

The three reference traces in
[`rework-r0-reference-traces.md`](rework-r0-reference-traces.md) exercise this
fold for Operator, Graph, and Connector Call hosts.

## 14. Repository-contact corrections

The opening specification was checked against the 2026-09-05 tree. Four
clarifications are necessary and are incorporated above:

1. **Graph lifecycle records:** Graph `l:start`/`l:stop` are currently empty
   `TransactionRecord` values committed by the lifecycle controller. “No Graph
   transaction” is restricted to application transactions (§2.1).
2. **Driver facts:** outbound Call completion is correlated to an existing
   OUTBOUND fact and does not become a new INBOUND root (§3).
3. **Recoverable manager commands:** a command must be journaled in its producer
   transaction before a later manager-consumption fact (§8.1). A single fold
   transition could not recover the required crash window.
4. **Proof plus explicit fuel:** a measure does not make `Loop.expired`
   impossible when a smaller explicit cap can win (§11.4).

The exact evidence and line references are recorded in
[`rework-r0-repository-audit.md`](rework-r0-repository-audit.md).

## 15. R0 closure

R0 is closed when this algebra, the three traces, repository audit, and v16
identity ledger agree. It does not authorize source, HIR, plan, or runtime
changes. Those begin only at R3 after the R0–R2 review stop.
