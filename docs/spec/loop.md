# Loop — bounded tail-recursive iteration

> **Status: Normative — under review.**
> Companion to [aspects.md](aspects.md), which defines the generic aspect
> mechanism, and [effects-and-sagas.md](effects-and-sagas.md), which defines
> Saga composition.

The `Loop` aspect re-applies a Graph, Operator, or Connector to a passed
envelope. It covers three related cases:

- counted work: apply a body a known number of times;
- convergence: refine data until a declared condition holds;
- retry: repeat an external call while an explicitly selected typed
  outcome case satisfies graph policy.

The graph remains acyclic. Re-entry is performed by the Loop manager after
one call has completed; it is not a graph edge and it does not suspend a
stack frame. The runtime model is tail recursion over immutable envelopes.

Every Loop declares exactly one qualified attribute:

```rastok
Loop.variant = {
  kind = measure
  initial = 10
  next = value - 1
  until = value <= 0
}
```

`variant` is therefore a property of the `Loop` aspect. It is not an
Operator `var`, a special top-level clause, or the dynamic data type named
`variant`.

## 1. Where Loop applies

The `: Loop` annotation is legal on every Node family:

```rastok
// Graph: the whole sub-DAG is applied again.
graph refine : Loop {
  input in: struct { error_margin: double, remaining_steps: int }

  Loop.variant = {
    kind = measure
    source = data.remaining_steps
    until = data.error_margin < 0.001
    limit = 100
  }

  . -> normalize -> enrich -> score -> .
}

// Operator: one pure handler is applied again.
operator repeat : Loop {
  Loop.variant = {
    kind = measure
    initial = 10
    next = value - 1
    until = value <= 0
  }

  on in { out = . }
}

// Connector: an external call is retried.
connector pay : HttpClient + Loop {
  url = "https://payments.example"

  Loop.variant = {
    kind = measure
    initial = 5
    next = value - 1
    until = value <= 0
  }
  Loop.backoff = {
    curve = exponential
    initial = duration("100ms")
    max = duration("5s")
  }

  on out where meta.ext.http_status < 500
  on out where meta.ext.http_status >= 500 { }
  on exit where data is ConnectFailed || data is TimeoutExpired { }
}
```

| Carrier | What one application means |
|---|---|
| Graph | one complete traversal from its input boundary to its output boundary |
| Operator | one completed handler transaction |
| Connector | one completed external call |

Each application must produce at most one continuation envelope. A Loop
whose iterating path emits none or more than one is rejected statically when
possible and emits a typed Loop exit case otherwise.

## 2. The `Loop.variant` measure

The value of `Loop.variant` must be a `measure` tagged value with these
fields:

| Field | Rule | Meaning |
|---|---|---|
| `initial` | exactly one of `initial` or `source` | the measure before the first application |
| `source` | exactly one of `source` or `initial` | a Rastok expression evaluated on the entering envelope and on each continuation envelope |
| `next` | required with `initial`; forbidden with `source` | a Rastok expression producing the next measure after an application |
| `until` | required | a Rastok boolean expression checked before each application |
| `limit` | required unless a finite bound is proved statically | maximum number of body applications |

Inside the block, `value` is the current measure. The current envelope is
also in scope, so `source` and `until` may read `data` and writable `meta`.
Elsewhere in the Loop's lexical body, the same read-only value is available
as `Loop.value`.

`initial`, `source`, `next`, `until`, and `limit` are ordinary Rastok
expressions. They cannot read a clock, perform I/O, or mutate state.

### 2.1 Counted form

```rastok
Loop.variant = {
  kind = measure
  initial = 10
  next = value - 1
  until = value <= 0
}
```

The predicate is checked before the body. This example therefore applies the
body ten times, with values 10 through 1, and exits when the next value is 0.

The compiler may derive the bound when all of the following are evident:

- `initial` is an integral constant;
- `next` moves by a non-zero integral constant towards the terminal range;
- `until` is a compatible comparison against an integral constant.

All other forms declare `limit` explicitly.

### 2.2 Envelope-derived form

`source` makes the body responsible for carrying the next measure in its
output:

```rastok
graph drain : Loop {
  input in: struct {
    items: array<variant>
    remaining: int
  }

  Loop.variant = {
    kind = measure
    source = data.remaining
    until = value <= 0
    limit = 10000
  }

  . -> pop_head -> .
}
```

`source` is evaluated first on the entering envelope, then on each
continuation envelope. The source expression is an obligation position:
its fields must be statically present or explicitly optional.

### 2.3 Decrease and bound checks

Before re-entry, the Loop manager checks:

1. `until` is false;
2. the call count is below `limit`, when a limit is present;
3. the next measure is strictly smaller than the current measure.

A non-decrease, an incomparable value, or a type change emits
`LoopVariantViolation`. Reaching the call limit without satisfying `until`
emits `LoopExhausted`. These checks are journaled and replay identically.

The current specification defines numeric measures. Lexicographic tuples
and user-defined well-founded orders are deferred.

### 2.4 Scope

`value` exists only inside the `measure` block. `Loop.value` is visible
inside the lexical body of the Node carrying `: Loop`, including nested
Operators declared there. Wiring an externally declared Node into the Loop
does not grant it access.

The measure is runtime-owned state. User code can read it but cannot assign
to it. The only ways to produce the next value are `next` or the next
envelope evaluated by `source`.

## 3. Exit semantics

Loop has three exit paths:

| Event | Result |
|---|---|
| `until` becomes true on a normal Graph or Operator iteration | current envelope leaves on `out` |
| a Connector completion reaches a body-less outcome handler | the original typed completion outputs leave unchanged |
| the measure exhausts or violates its invariant | `LoopExhausted` or `LoopVariantViolation` leaves on the qualified typed `Loop.exit` output |

An `on <output> where <predicate> { ... }` handler with a body is an
iterating case. `<output>` is an ordinary typed port selected by the
author (`exit` for Process); no port name and no common `transient`
field has privileged meaning. The Loop judges the boundary completion as
one unit so simultaneous `out`/`err`/`exit` facts cannot be split by
scheduler order. If an iterating case requests another call after the
measure is exhausted, the Loop emits
`LoopExhausted { calls, last_outcome }` on its declared `Loop.exit` output.

An unmatched outcome follows the ordinary resolved sink/wiring policy;
Loop never turns technical exhaustion into a business outcome.

## 4. Backoff

`Loop.backoff` is optional and applies between applications:

```rastok
Loop.backoff = {
  curve = exponential
  initial = duration("100ms")
  factor = 2.0
  max = duration("5s")
}
```

The initial catalog contains `constant`, `linear`, `exponential`, and
`fibonacci` tagged values. Their fields are compile-time constants. With no
backoff, re-entry is immediate.

A delay is a dated runtime event. Production execution is driven by the
system Clock; Lab execution is driven by virtual time and `advance`.
The obligation is scoped to the current Node-instance generation. If that
instance enters Closing before the delay fires, the pending `l:next` becomes
stale and cannot create work in a later generation. A Loop wakeup is not a
Connector call and therefore owes no invocation `exit` of its own.

## 5. Lifecycle hooks

Loop lifecycle hooks use qualified names:

```rastok
on Loop.start { started = . }
on Loop.stop  { finished = . }
```

`Loop.start` fires once before the first predicate check. `Loop.stop` fires
once after any terminal result: success, selected base outcome, exhaustion, or variant
violation. Bare `on start` and `on stop` remain Node lifecycle hooks.

## 6. Envelope context

Envelopes born inside a Loop carry runtime-owned `meta.sys` birth stamps:

- `loop_id`: identity of the current Loop run;
- `loop_stack`: enclosing Loop identities;
- `iter_count`: zero-based application count.

These fields cannot be written by DSL code. Re-entry emits a fresh envelope
with trigger `l:next` and the previous application's output as causal parent.

## 7. Composition with Saga

Loop and Saga are independent aspects and may be combined on one Connector
or nested lexically.

### 7.1 Same-node composition

```rastok
connector reserve : HttpClient + Saga + Loop {
  effect = compensable

  Loop.variant = {
    kind = measure
    initial = 5
    next = value - 1
    until = value <= 0
  }

  on out where meta.ext.http_status < 500
  on out where meta.ext.http_status >= 500 { }
  on exit where data is ConnectFailed || data is TimeoutExpired { }
  on compensate { /* inverse request */ }
}
```

Loop resolves the call attempt. Saga records a step only after Loop returns
a successful result. Exhaustion is one failed Saga step, not five completed
steps.

### 7.2 Nested composition

A Loop inside a Saga step is sealed before the Saga advances. A Saga inside
a Loop iteration is likewise sealed before re-entry. Aspect stacks are
lexical and journaled; they do not interleave implicitly.

### 7.3 Effect rules inside Loop

Retrying an effectful Connector is legal only when its external contract
makes the repeated call explicit:

- `none` and `idempotent` effects may repeat;
- `compensable` and `irreversible` effects require an
  `idempotency_key`;
- the same byte-identical business key is normally reused for every attempt
  of one logical call.

A per-attempt key is an explicit escape for a partner that permanently
consumes a key on a processed failure. It changes the business operation
seen by the partner and can double-apply after a late success, so it must be
visible in source and in the coverage report. The full key-stability rule
lives in [delivery-semantics.md](delivery-semantics.md) §4.3.

## 8. Static rules

The validator enforces:

1. `Loop.variant` appears exactly once on every `: Loop` Node and nowhere
   else.
2. Its value is a `measure` block with the field combinations in §2.
3. `until` is boolean; `limit`, when present, is a positive integral
   constant.
4. The measure type is stable across applications.
5. A form without a statically derived bound declares `limit`.
6. An iterating path has at most one continuation envelope.
7. Loop-owned fields and `meta.sys.loop_*` are read-only.
8. Effectful retries satisfy §7.3.
9. Re-entry is runtime-owned and never creates a static DAG edge.

## 9. Limitations and open questions

- Static bound derivation initially covers only the counted form in §2.1.
- General proofs that a `source` decreases are deferred; runtime checking and
  the explicit `limit` remain authoritative.
- Tuple measures and custom orders are not part of the current type surface.
- Wall-clock deadlines are expressed by a Connector observation and an exit
  handler, not as a hidden Loop clock.

## 10. Prover mapping

The executable model maps directly to a recursive function:

| Rastok | Proof-oriented reading |
|---|---|
| Node carrying `: Loop` | tail-recursive function over an envelope |
| `Loop.variant` | decreasing measure |
| `until` | base-case predicate |
| body application | recursive step |
| `limit` | executable finite bound when no stronger proof is available |

Runtime checks provide executable evidence. They are not themselves a
mathematical proof of total correctness.

## 11. Lineage

The design follows the loop-variant tradition associated with Floyd and
Hoare, and the decreasing-measure formulations used by SPARK, Dafny, and
Coq. Rastok keeps the user-facing word `Loop` because the source describes
iteration; the runtime uses the equivalent tail-recursive execution model.

## 12. Termination guarantee

Every admitted Loop has a finite call bound:

- the compiler derives it for a tractable counted measure; or
- the declaration supplies `limit`.

Strict-decrease checking catches a broken progress claim before the next
application. The hard bound remains the final guard. Rastok therefore
guarantees that the Loop manager will stop; it does not claim that an
external Connector will answer before its separately declared timeout.

## 13. Prover direction

A future theorem layer may discharge a derived bound or prove that a
`source` measure decreases. That proof can remove redundant runtime checks,
but it must not change observable graph semantics. Until then, the
executable checks in §2.3 and §12 are normative.

## 14. Related documents

- [aspects.md](aspects.md) — annotation, qualification, and composition;
- [effects-and-sagas.md](effects-and-sagas.md) — effect and Saga rules;
- [execution-model.md](execution-model.md) — handlers, timeouts, and dated
  events;
- [delivery-semantics.md](delivery-semantics.md) — journal and stable
  idempotency keys;
- [envelope.md](envelope.md) — Loop birth stamps;
- [grammar.md](grammar.md) — concrete syntax.

## 15. First dischargeable theorem: bounded retry

For the counted declaration

```rastok
Loop.variant = {
  kind = measure
  initial = 5
  next = value - 1
  until = value <= 0
}
```

assume every external attempt terminates with a result or its declared
timeout. The measure starts at 5, decreases by 1 after every retry, and the
terminal predicate holds at 0. The Connector therefore performs at most
five attempts for one logical call. This is the first Loop property the
future theorem layer should be able to discharge automatically.
