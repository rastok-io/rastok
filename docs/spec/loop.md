# Loop — finite temporal recursion

> **Status: normative M1.7 target.** The source classifier recognizes
> `Loop` as a `node::Aspect`, but the checker remains fail-closed until the
> categorical plan and runtime gates described here land. No accepted program
> may silently execute as an ordinary Graph while carrying `: Loop`.

`Loop` applies one Graph repeatedly to an immutable iteration-state envelope.
It is finite temporal recursion, not a spatial cycle:

```text
external in ──▶ body ──▶ out       terminal result
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

A Loop Graph has four conventional port roles:

| Port | Direction | Contract |
|---|---|---|
| `in` | input | state envelope for the current iteration |
| `next` | output owned by Loop | next state; its logical type is exactly the type of `in` |
| `out` | output | terminal business result (`Done`) |
| `expired` | output owned by Loop | `LoopExpired { last_state: In }` |

`next` and `expired` are contributed by the aspect and become ordinary typed
plan ports after checking. User code emits them like any other port, but they
cannot be redeclared with another direction or type. `in` and `out` remain
ordinary Graph boundary declarations so their types stay visible.

Every iteration makes **exactly one control decision**:

```text
next(state) XOR out(result)
```

The checker proves this over all reachable handler paths. A path that emits
neither, both, or more than one of either is rejected. The VM repeats the same
check only as defense against a forged or corrupted plan; it never guesses a
decision from scheduler order. Other diagnostic or audit outputs may be
emitted atomically, but they do not decide the Loop. Multiple business terminal
cases use one tagged-union `out` type.

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
| `Loop.fuel` | positive plan-constant integer | required until a `Loop.measure` proof is discharged |
| `Loop.delay` | closed plan-constant delay policy | optional; absence means immediate re-entry |
| `Loop.invariant` | boolean theorem expression | optional theorem input; deferred in the first executable gate |
| `Loop.measure` | well-founded theorem expression | optional termination witness; deferred in the first executable gate |
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
    out(result)  -> out(result)
    next(state') -> run(N, state')
```

Consequently, if the last permitted body application emits `next`, the
manager does **not** commit an `l:next`. It emits
`LoopExpired { last_state = state' }`: the proposed next state is the most
truthful terminal observation.

Fuel is derived from the committed Loop fold, never from a mutable RAM counter.
Recovery cannot regain an application. Duplicate or stale wakeups cannot spend
fuel twice.

Fuel proves a finite number of iterations. It does not prove that one
iteration returns. Every admitted body must therefore be total under its own
contract:

- pure Rastok expressions are total or fault the transaction;
- an outbound Call has a positive finite `timeout`;
- a guest execution has its declared finite budget;
- an open-ended Event or Session wait is rejected from the M1.7 body.

## 5. Delay and causal history

Immediate and delayed recurrence use the same journal fact. After a committed
`next(state')`, the Loop manager records `l:next` with:

- the current `LoopRunRef`;
- the next iteration ordinal;
- the remaining fuel;
- `state'` as the re-entry envelope;
- the `next` emission as causal parent;
- the due date, when `Loop.delay` is present.

The generic dated-event queue uses timer kind 5. Lab time advances only through
`advance`; production uses the hosted Clock. Equal input facts and equal
virtual time therefore produce the same causal core.

A pending Loop wakeup is scoped to the current instance generation and counts
as an armed obligation for `idle`. If the owner enters Closing, that wakeup is
cancelled/stale and cannot open work in a successor generation. A Loop wakeup
is not a Connector call and owes no invocation `exit`.

## 6. Termination witnesses

An admitted Loop has at least one termination witness:

| `Loop.fuel` | discharged `Loop.measure` | Admission |
|---:|---:|---|
| present | absent | yes — structural finite bound |
| absent | present | yes — theorem proves termination |
| present | present | yes — theorem plus operational cap |
| absent | absent | reject |

The first executable M1.7 gate admits the first row. `Loop.invariant` and
`Loop.measure` remain named, fail-closed theorem positions until the verifier
can produce and the core can check a proof certificate. They never degrade
into runtime assertions masquerading as proof.

### 6.1 Proof obligations

For state `s`, invariant `I`, measure `m` and every reachable path through the
finite body, the verifier must establish:

1. `I(s)` at entry;
2. one body application terminates;
3. exactly one of `next(s')` or `out(r)` is emitted;
4. every `next(s')` preserves `I(s')`;
5. every `next(s')` strictly decreases `m(s') < m(s)` in a well-founded order.

These obligations prove eventual `out`; no independent `until` predicate is
needed. An external Connector result is a fresh symbolic typed value. The
verifier may use explicit guards on that value, but never assumes that a
remote service progresses merely because it usually does.

The initial automatic fragment is intentionally small: affine integer
arithmetic, comparisons, boolean connectives, structs and tagged unions over a
finite spatial DAG. Unsupported guest calls, mutable state interactions,
opaque functions, open Sessions or unproved nested summaries refuse the proof
by name. The hosted compiler may search for a proof; a small `no_std` core
checker validates the certificate. A future Rocq/Coq export is an independent
oracle, not the runtime authority.

The name `measure` follows Rocq/Coq's well-founded recursion vocabulary
(`{measure term relation}` / `{wf relation term}`). `variant` remains a data
type term in Rastok and is not reused for termination.

## 7. Examples

### 7.1 Count down

```rastok
graph countdown : Loop {
  input in: struct { n: int }
  output out: text

  Loop.fuel = 100

  on in where data.n <= 0 { out = "liftoff" }
  on in where data.n > 0  { next = { n = data.n - 1 } }
}
```

### 7.2 Fold a finite payload

```rastok
graph total : Loop {
  input in: struct { items: array<int>; i: int; acc: int }
  output out: int

  Loop.fuel = 10000

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

### 7.3 Retry an outbound Call

```rastok
graph pay : Loop {
  input in: PaymentRequest
  output out: union { Receipt, Declined }

  Loop.fuel = 5
  Loop.delay = {
    kind    = exponential
    first   = duration("100ms")
    ceiling = duration("5s")
  }

  connector charge : HttpClient {
    url      = "https://api.example/pay"
    effect   = idempotent
    behavior = probabilistic
    timeout  = duration("10s")
  }

  . -> charge
  on charge.out  { out = Receipt { value = data } }
  on charge.exit where data is TimeoutExpired {
    next = Loop.state.data
  }
  on charge.exit where data is Exited {
    out = Declined { status = data }
  }
}
```

The retry policy belongs to the Graph, not to `HttpClient`. Stable
idempotency keys and call close-once remain Connector obligations. A
`Process(codec = jsonl)` can emit several `out` frames, so it is admitted only
when the surrounding Graph still proves exactly one control decision.

## 8. What is not a Loop

| Need | Primitive | Why not Loop |
|---|---|---|
| aggregate independent arrivals | Node-private `var` + optional `instance`/`lifetime` | state lives across unrelated root envelopes |
| global or keyed window | finite `lifetime` + `on stop` | time closes an instance, not recursive progress |
| calendar recurrence | `Calendar` | there is no result toward which one run converges |
| open-ended polling daemon | `Clock` + ordinary Graph | intentionally unbounded service |
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
2. `in` and `out` exist and `next`/`expired` have their exact contributed
   types;
3. `Loop.fuel` is positive when no checked proof certificate exists;
4. `Loop.delay` is canonical and all durations are positive;
5. every reachable iteration path decides exactly one of `next`/`out`;
6. the body is individually terminating under its Connector/guest budgets;
7. no spatial edge reads or writes `next`;
8. run identity, fuel and causal parents agree with the journal fold;
9. instance Closing cannot revive a stale wakeup.

Any unsupported theorem expression, carrier, aspect combination or body
cardinality refuses before VM construction.

## 11. Related documents

- [aspects.md](aspects.md) — Node kind/type/aspect classification;
- [instances.md](instances.md) — key, lifetime, generation and Closing;
- [execution-model.md](execution-model.md) — transactions and dated events;
- [delivery-semantics.md](delivery-semantics.md) — journal-before-effect and
  Connector close-once;
- [identity.md](identity.md) — extend-only causal registries;
- [grammar.md](grammar.md) — concrete source productions.
