# Loop — finite temporal recursion

> **Status: normative and executable through M1.7 L7.** Fuel-bounded and
> directly ranked `Loop`s cross source, checked HIR and categorical v14 with
> their exact boundary and ordinary Graph-owned
> handlers. Registry-free validation re-derives the contract, finite progress
> decision and proof certificate after decode. The VM
> executes a root Loop whose traversal lives in Graph-owned handlers and the
> flattened finite DAG of ordinary nested Graphs, including handlers of direct
> child outputs, with or without `Loop.delay`, under the ordinary instance
> lifetime profiles. Direct Graph-boundary terminal
> producers, nested Loop owners, spatial Worker recovery and explicit `stop`
> remain named, fail-closed composition gates. No accepted program silently
> executes as an ordinary Graph.

`Loop` applies one Graph repeatedly to an immutable iteration-state envelope.
It is finite temporal recursion, not a spatial cycle:

```text
external in ──▶ body ──▶ result / exit / stop    terminal
                    └──▶ next ─┐
                         l:next│
                    in ◀───────┘
```

The declared topology remains a DAG. `next` commits a dated or immediate
`l:next` fact and re-enters the same Loop run; it is never a graph edge.

M1.7 admits `Loop` on a **Graph only**. Pure Operator work and Connector calls
remain ordinary children of that Graph. This is not a loss of expressivity:
it gives every Loop one boundary, one state type and one totality proof instead
of three carrier-specific protocols. Direct `Operator + Loop` and
`ConnectorType + Loop` are named later gates.

## 1. Boundary contract

A Loop Graph has exactly one input, `in`, two aspect-owned outputs, and any
number of ordinary terminal outputs chosen by its domain:

| Port | Direction | Contract |
|---|---|---|
| `in` | input | state envelope for the current iteration |
| `next` | output owned by Loop | next state; its logical type is exactly the type of `in` |
| any ordinary output (`out`, `exit`, `declined`, …) | output | terminal business result or observation |
| `expired` | output owned by Loop | `LoopExpired { last_state: In }` |

`next` and `expired` are contributed by the aspect and become ordinary typed
plan ports after checking. They cannot be redeclared with another direction or
type. User code may emit `next`; `expired` is manager-produced only. `in`
remains an ordinary Graph boundary declaration so its type stays visible, but
no second Graph input is admitted: another asynchronous stimulus belongs on
an ordinary child Connector and must still lead to one proved verdict.
`out` has no privileged Loop meaning: when declared, it is one terminal output
among the Graph's ordinary outputs.

Every completed iteration must make **exactly one progress decision**:

```text
continue:  exactly one next(state), no ordinary output, no stop
terminate: no next, at least one ordinary output and/or stop
```

L3 proves this over all reachable handler paths. Silence, repeated `next`,
more than one terminal transaction, or continuation mixed with termination
is rejected. Registry-free validation repeats the same proof after decode as
defense against a forged or corrupted plan; the runtime never guesses a
decision from scheduler order. A single terminal transaction may emit several
ordinary outputs atomically. If an application needs a single result sum, it
may still use one tagged-union `out`; Loop does not require it.

Bare `stop` is a stronger terminal decision: it closes the current Loop owner
instance, so every run in that instance follows the ordinary Closing protocol.
It is not merely a private return from one run. A run that should finish while
leaving sibling runs alive emits an ordinary terminal output instead. If the
owner subtree can hold an outbound call, writing `stop` also makes the existing
`closing = { drain_for; settle_for }` policy required; no grace period is
invented silently.

`next` is an aspect-owned control port. It may be connected only to the Loop
manager selected by the checker; source wiring to or from it is rejected. The
`expired` port is ordinary after contribution and follows the usual wiring and
root-sink policy.

## 2. Surface

Aspect-owned attributes use their canonical qualified spelling. The prefix is
not optional: qualification keeps aspect composition local and prevents a
future `Saga.fuel` or Connector `delay` from changing meaning.

| Attribute | Phase and type | Presence |
|---|---|---|
| `Loop.fuel` | positive plan-constant integer | optional when `Loop.measure` is independently proved; when both exist, this is the operational cap |
| `Loop.delay` | closed plan-constant delay policy | optional; absence means immediate re-entry |
| `Loop.invariant` | boolean theorem expression | optional with `Loop.measure`; L6 admits absence or literal `true` and refuses every richer claim |
| `Loop.measure` | signed-integer ranking expression | optional termination witness; L6 admits a direct path rooted at iteration `data` |
| `Loop.jitter` | deterministic delay policy | reserved and rejected by name |

The closed delay forms are:

```rastok
Loop.delay = {
  kind  = fixed
  after = duration("100ms")
}

Loop.delay = {
  kind    = exponential
  first   = duration("100ms")
  ceiling = duration("5s")
}
```

All durations are positive plan constants. For the `k`th re-entry, where
`k >= 1`, exponential delay is:

```text
min(ceiling, first * 2^(k - 1))
```

Arithmetic saturates safely at `ceiling`. Delay changes only the admission
date of `next`; it does not add fuel, extend a Connector timeout or alter the
instance lifetime.

The following former spellings are removed rather than aliased:
`Loop.variant`, `Loop.backoff`, `initial`, `source`, `value`, `until`,
`limit`, `max_steps`, and `Loop.decreases`. Each receives one migration
diagnostic to `Loop.fuel`, `Loop.delay`, or `Loop.measure` as appropriate.

## 3. Iteration state

At every iteration, `data` is the current input payload. The read-only binding
`Loop.state` preserves the complete entry envelope even if a handler later
works with another envelope:

```text
Loop.state.data
Loop.state.meta
```

That is the only Loop-specific expression binding. There is no separate
`initial`, `value`, attempt variable or writable Loop state. User `var` cells
remain private to their declaring Node and follow the ordinary instance rules.

One external root envelope opens one **Loop run**. Its `LoopRunRef` is derived
from that root envelope identity. The run carries its own fuel, iteration
ordinal and pending wakeup. Several runs may coexist in one keyed Graph
instance and may interleave while one is delayed; mailbox serialization still
makes each committed Graph transaction atomic.

An iteration does not create a new instance generation. Instance generation
belongs to `l:start`/`l:stop`; Loop progress belongs to the `LoopRunRef` and
its zero-based iteration ordinal.

## 4. Fuel and exhaustion

`Loop.fuel = N` bounds **body applications**, not re-entries. Its operational
meaning is exact:

```text
run(0, state) = expired(LoopExpired { last_state = state })

run(N + 1, state) =
  body(state) then
    terminal(outputs and/or stop) -> terminal
    next(state')                  -> run(N, state')
```

Consequently, if the last permitted body application emits `next`, the
manager does **not** commit an `l:next`. It emits
`LoopExpired { last_state = state' }`: the proposed next state is the most
truthful terminal observation.

Fuel is derived from the committed Loop fold, never from a mutable RAM counter.
Recovery cannot regain an application. Duplicate or stale wakeups cannot spend
fuel twice.

For a proved Loop without explicit fuel, the manager evaluates the checked
measure once against the immutable root input and derives the internal body
budget `max(measure, 0) + 1`. This budget is an execution carrier, not a second
termination claim and not source identity: the certificate has already proved
that each recurrent decision decreases the measure while it is positive. When
both witnesses are present, explicit `Loop.fuel` wins as the deliberate
operational cap.

L4 records one closed, extend-only fact in transaction key 53:

```text
Open    (run, instance, root, fuel, state₀)
Next    (run, iteration, remaining, selected-next, stateₙ)
Admit   (run, iteration)
Close   (run, iteration)
Expired (run, iteration, selected-next, LoopExpired envelope)
```

`Open` is unique for a run. `Next` must advance the ordinal by exactly one and
reduce remaining fuel by exactly one; its re-entry envelope is derived from
the selected `next` emission. `Admit` is the exactly-once temporal crossing
which makes that state eligible for one body application; it carries no user
emission or mutable state. `Close` and `Expired` are terminal and reject any
later progress. The complete live projection is a fold over these facts, not a
second authority. A journal implementation may maintain a replay-derived
index of that projection, provided a fresh fold produces the same state. On a
fresh Worker boot, open direct-body runs rebuild their disposable execution
work from that projection; the journal remains the only source of the current
state, ordinal and fuel.

Fuel proves a finite number of iterations. It does not prove that one
iteration returns. Every admitted body must therefore be total under its own
contract:

- pure Rastok expressions are total or fault the transaction;
- an outbound Call has a positive finite `timeout`;
- a guest execution has its declared finite budget;
- an open-ended Event or Session wait is rejected from the M1.7 body.

## 5. Delay and causal history

Immediate and delayed recurrence use the same pair of journal facts. After a
committed `next(state')`, `Next` records:

- the current `LoopRunRef`;
- the next iteration ordinal;
- the remaining fuel;
- `state'` as the re-entry envelope;
- the `next` emission as causal parent;
- the due date as a persisted non-causal scheduler annotation when
  `Loop.delay` is present.

Canonical key 53 keeps the same `next` array in both cases. A delayed
transaction seals timer tuple `(5, reentry-envelope-id)` in key 37; the due
instant remains outside semantic identity but is returned by the journal's
Loop fold so recovery can re-arm it. Lab time advances only through `advance`;
production maps its hosted time source to the same generic dated-event queue.
Equal input facts and equal virtual time therefore produce the same causal
core.

When the date is due (or immediately when no delay exists), `Admit` is the
`l:next` transaction. It selects the body's ordinary spatial routes from the
recorded state, proves bounded mailbox capacity, commits those routes, and only
then makes the deliveries visible. Capacity is never held while a delay is
pending. Direct Graph handlers use the same empty-route admission, so there is
one recurrence protocol rather than a direct-body fast path with different
semantics.

A direct child output may activate one Graph-owned handler only after the
child's producer transaction commits. The handler's decision must remain in
the current iteration state's causal lineage. This supports bounded outbound
Calls: their own timeout first publishes the typed `exit` which chooses
`next`; `Loop.delay` is armed from that later decision and cannot alter the
Call deadline.

A pending Loop wakeup is scoped to the current instance generation. It is an
armed obligation, but it does not postpone the independent `idle`, `fixed` or
`tumbling` deadline. If such a deadline wins, the owner enters Closing and the
still-dated wakeup becomes stale; it cannot open work in a successor
generation. A wakeup already admitted before Closing may drain until the
closing drain deadline. A Loop wakeup is not a Connector call and owes no
invocation `exit`.

## 6. Termination witnesses

An admitted Loop has at least one termination witness:

| `Loop.fuel` | discharged `Loop.measure` | Admission |
|---:|---:|---|
| present | absent | yes — structural finite bound |
| absent | present | yes — theorem proves termination |
| present | present | yes — theorem plus operational cap |
| absent | absent | reject |

All four rows are now enforced. A proof-bearing source is accepted only after
the hosted checker derives a certificate and the small `no_std` core can
derive the same evidence from the canonical handler table. Categorical v14
transports the checked measure, optional invariant and sorted multiset of exact
decrements. Decode validation ignores that list as authority: it re-runs the
verifier and requires exact agreement. These expressions never degrade into
runtime assertions masquerading as proof.

### 6.1 Proof obligations

For state `s`, invariant `I`, measure `m` and every reachable path through the
finite body, the verifier must establish:

1. `I(s)` at entry;
2. one body application terminates;
3. exactly one progress verdict is produced: one `next(s')`, or explicit
   termination by ordinary output(s) and/or `stop`;
4. every `next(s')` preserves `I(s')`;
5. every `next(s')` strictly decreases `m(s') < m(s)` in a well-founded order.

With fuel, the structural budget proves a finite number of body applications,
including a run that exhausts onto `expired`; they do not promise a particular
terminal port. With a standalone `measure`, strict decrease additionally
proves that a terminal verdict is eventually reached. No independent `until`
predicate is needed. An external Connector result is a fresh symbolic typed
value. A later verifier may use explicit guards on that value, but must never
assume that a remote service progresses merely because it usually does.

The L3 **control** fragment is intentionally small: boolean literals and
connectives; a proposition paired with its syntactic negation; complementary
`==`/`!=`, `<`/`>=` and `<=`/`>` comparisons over checked expression trees;
and exhaustive `data is Case` partitions over a closed tagged union. The
summary follows ordinary Operators through the finite spatial DAG. Ordinary
nested Graph boundaries are passive composition seams and disappear during
lowering, so their descendant Nodes participate in that same proof. This does
not admit a nested Graph carrying its own `Loop` aspect or create a recursive
Graph summary. A bounded
outbound Call may decide only from its exactly-once typed `exit`; a zero-or-more
output cannot decide progress. Mutable Graph state, open Event/Session waits,
a nested Loop owner, or a proof exceeding the finite proposition budget refuse
by name.

The L6 **termination** fragment is deliberately narrower and closed:

- the measure is a direct signed-integer path, `data` or `data.field...`;
- every recurrent handler is a direct Graph `on in` handler;
- its guard contains `measure > 0`, `measure >= 1`, or the equivalent reversed
  comparison, possibly as one conjunct;
- its one `next` writes the measured scalar or struct field as exactly
  `measure - k`, where `k` is a positive integer literal;
- an invariant is either absent or literal `true`.

The verifier records the sorted multiset of all such positive `k` values. It
retains duplicates, so handler multiplicity cannot disappear under sorting.
Last-write semantics are honored: a later write that obscures the measured
field invalidates an earlier apparent decrease. Any child-output recurrence,
Connector-dependent progress, computed measure, mutable-state argument,
non-trivial invariant, lexicographic ranking or opaque expression refuses with
one `E-LOOP`; it is not approximated. Later theorem gates may extend this
certificate algebra with affine SSA, tagged-union case reasoning and
proof-carrying summaries for nested recurrence. A Rocq/Coq export remains an
independent oracle, not the runtime authority.

The name `measure` follows Rocq/Coq's well-founded recursion vocabulary
(`{measure term relation}` / `{wf relation term}`). `variant` remains a data
type term in Rastok and is not reused for termination.

## 7. Examples

### 7.1 Count down

```rastok
graph countdown : Loop {
  input in: struct { n: int }
  output out: text

  Loop.measure = data.n

  on in where data.n <= 0 { out = "liftoff" }
  on in where data.n > 0  { next = { n = data.n - 1 } }
}
```

### 7.2 Fold a finite payload

```rastok
graph total : Loop {
  input in: struct { items: array<int>; i: int; acc: int }
  output out: int

  Loop.fuel = 10000 // the indexed fold is not in the first measure fragment

  on in where data.i >= size(data.items) { out = data.acc }
  on in where data.i < size(data.items) {
    next = {
      items = data.items
      i     = data.i + 1
      acc   = data.acc + data.items[data.i]
    }
  }
}
```

### 7.3 Retry a bounded outbound Call

```rastok
graph retry_job : Loop {
  input in: text
  output exit_code: int

  Loop.fuel = 5
  closing = { drain_for = duration("2s"); settle_for = duration("2s") }
  Loop.delay = {
    kind    = exponential
    first   = duration("100ms")
    ceiling = duration("5s")
  }

  connector job : Process {
    command  = "worker"
    codec    = lines
    effect   = idempotent
    behavior = probabilistic
    timeout  = duration("10s")
    max_output = "1 MiB"
  }

  . -> job
  job.out -> nothing
  job.err -> nothing
  on job.exit where data is TimeoutExpired  { next = Loop.state.data }
  on job.exit where data is Exited          { exit_code = data.exit_code }
  on job.exit where data is SpawnFailed     { stop }
  on job.exit where data is DecodeFailed    { stop }
  on job.exit where data is CaptureFailed   { stop }
  on job.exit where data is OutputOverflow  { stop }
  on job.exit where data is OutcomeUnknown  { stop }
}
```

The retry policy belongs to the Graph, not to `Process`. Stable idempotency
keys and call close-once remain Connector obligations. The exactly-once typed
`exit` union is a finite guard domain: timeout continues, a normal exit emits
a domain result, and the remaining cases deliberately close this singleton
Loop instance. A `Process(codec = jsonl)` can emit several `out` frames, so
those frames cannot each choose Loop progress. They are discarded here; a
later bounded collector may turn them into one ordinary terminal value.

## 8. What is not a Loop

| Need | Primitive | Why not Loop |
|---|---|---|
| aggregate independent arrivals | Node-private `var` + optional `instance`/`lifetime` | state lives across unrelated root envelopes |
| global or keyed window | finite `lifetime` + `on stop` | time closes an instance, not recursive progress |
| calendar recurrence | `Calendar` | there is no result toward which one run converges |
| open-ended polling daemon | `Timer` + ordinary Graph | intentionally unbounded service |
| long-lived process/session | Connector mode + `lifetime` | lifecycle, not recursion |

`lifetime` and Loop are orthogonal. A finite lifetime may close an instance
containing active Loop runs. The termination theorem is conditional on the
owner remaining open; Closing follows the instance protocol and invalidates
pending Loop wakeups.

## 9. Composition

`Saga + Loop` is not enabled merely because both names are recognized. Its
composition rule must prove which journal fold is outermost. The target rule
is: one complete Loop run resolves inside one Saga step, and only its terminal
result becomes the step outcome. Saga compensation debt survives instance
Closing; Loop wakeups do not.

## 10. Validation checklist

Before a Loop plan becomes executable, Registry-free validation re-derives:

1. carrier is a Graph and the aspect occurs once;
2. typed `in` exists and `next`/`expired` have their exact contributed types;
3. at least one finite witness exists: positive `Loop.fuel` and/or a checked
   proof certificate whose measure, invariant and recurrence evidence are
   re-derived from the transported handler table;
4. `Loop.delay` is canonical and all durations are positive;
5. every reachable iteration path decides exactly one of continuation or
   termination, independently of terminal-output spelling;
6. the body is individually terminating under its Connector/guest budgets;
7. no spatial edge reads or writes `next`;
8. run identity, fuel and causal parents agree with the journal fold;
9. instance Closing cannot revive a stale wakeup.

Any unsupported theorem expression, carrier, aspect combination or body
cardinality refuses before a validated plan is returned. L6 adds direct
signed-integer ranking proofs without changing L5's immediate/delayed runtime
protocol, causal facts or spatial-Call boundary. Direct Graph-boundary terminal
producers, nested Loop owners, spatial recovery and explicit `stop` remain
fail-closed until their named composition gate lands.

## 11. Related documents

- [aspects.md](aspects.md) — Node kind/type/aspect classification;
- [instances.md](instances.md) — key, lifetime, generation and Closing;
- [execution-model.md](execution-model.md) — transactions and dated events;
- [delivery-semantics.md](delivery-semantics.md) — journal-before-effect and
  Connector close-once;
- [identity.md](identity.md) — extend-only causal registries;
- [grammar.md](grammar.md) — concrete source productions.
