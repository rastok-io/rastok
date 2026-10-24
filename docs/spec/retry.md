# Retry — bounded repetition of one operation

> **Status: normative; [current format](identity.md).** Finite outbound Call
> Retry is executable. [D-2026-09-14-4](decisions.md#d-2026-09-14-4--separate-loop-retry-and-saga-over-one-recurrence-engine)
> separates Retry from Loop and Saga; the September 15 decisions fix original-plan
> idempotence, closure-based delay and Closing. Process modes share one Contract
> under [D-2026-10-22-1](decisions.md#d-2026-10-22-1--one-process-contract-two-interaction-modes-package-format-22).
> [D-2026-10-23-1](decisions.md#d-2026-10-23-1--one-rule-for-the-l-registry-lloop-and-lclose-package-format-23)
> names the manager trigger `l:retry`. This document gathers those existing rules;
> the dated [C1 algebra](../architecture/retry-c1-algebra.md) and
> [C4 qualification](../architecture/retry-c4-qualification.md) remain evidence
> of their design and implementation, not a second current specification.

## 1. Contract at a glance

A Loop advances a state; a Retry repeats one prepared operation. Loop proves
finite iteration through fuel or a decreasing measure. Retry proves safe
repetition through terminal visibility and idempotence, and bounds it through
a total attempt count. Neither adds a spatial edge to the DAG.

`Retry` is an [Extension](extensions.md), not a Node, handler or Connector
Contract. It contributes three attributes, no port, no command, no state
binding and no new terminal case. The manager retains the request; the body
does not choose a replacement state. The final failure passes through unchanged,
without a `RetryExpired` wrapper.

Retry does not promise that an external operation has stopped when its local
Call times out. Reentrancy, compensation and business deduplication are distinct
properties; none supplies an implicit repetition guarantee.

## 2. Source and admission

This fragment shows a policy; the ordinary HttpClient request, codec, behavior
and positive timeout attributes are still required:

```rastok
connector pay : HttpClient + Retry {
  idempotent = true
  idempotency_key = meta.app.payment_id
  Retry.attempts = 3
  Retry.on = [ConnectFailed, TimeoutExpired, ServerError]
  Retry.delay = {
    kind = exponential
    first = duration("100ms")
    ceiling = duration("2s")
  }
  // Ordinary HttpClient settings follow.
}
```

| Attribute | Rule |
|---|---|
| `Retry.attempts` | required positive plan-constant integer; includes the first Call |
| `Retry.on` | required static set of resolved Contract terminal names; duplicates and unknown cases refuse |
| `Retry.delay` | optional fixed or exponential logical delay; positive plan-constant durations, no jitter |

A fixed delay is `{ kind = fixed; after = duration("100ms") }`. An exponential
delay has `first <= ceiling`. There is no `Retry.cases` alias, arbitrary
payload predicate, executable decision handler or expression-valued case set.
Qualified attributes require the owner's `+ Retry` declaration.

Admitted hosts are finite outbound Calls with bounded completion capture and a
positive plan-constant timeout: Process **job**, File **read/replace**, and
HttpClient. Operator, Graph, inbound Call, Event, Session and Process **service**
Retry refuse by name. A Loop Graph may contain Retry Connectors; `Loop + Retry`
on the same owner is not admitted. Process's mode changes its exchange, not
the fact that Retry requires a Call.

### 2.1 Repetition safety

For every case in `Retry.on`, the checker and independent plan validator require:

```text
safe = terminal visibility is before_effect OR original_plan.idempotent
```

An `outcome_unknown` or `after_effect` case therefore requires `idempotent = true`.
The guarantee belongs to the original validated plan, never a later source file,
Call fact or Driver claim. A partner key alone proves nothing. `behavior` is
the determinism axis, not repetition safety; the removed `effect` attribute
does not supply a guarantee.

Process job and HttpClient accept an authored plan-constant boolean; omission
means no guarantee. File read/replace fixes the same-request guarantee to true
and refuses a contradictory override. This does not promise that two reads
observe unchanged file contents.

### 2.2 HTTP responses

The [HttpClient Contract](stdlib.md#32-httpclient--outbound-http) classifies a usable decoded
500–599 response as `ServerError { status }`. Other admitted final statuses
produce `Completed { status }`. Both are `after_effect` and produce one ordinary
response-body `out` before `exit`; neither is a transport failure. Decode,
protocol and body-limit failures retain their distinct cases.

`Retry.on = [ServerError]` selects all 5xx, not only 500. The idempotence rule
still applies. `Completed` is not business success: 4xx, including 429, remain
there and are not automatically retried. The graph can classify the final
published result through ordinary Dispatch.

## 3. One operation, bounded attempts

An operation belongs to an exact opening delivery, entry and instance
generation. Two fan-out deliveries of the same envelope open distinct
operations, even if their business origin or partner key is equal.

Before the first effect, pure preparation fixes the original envelope,
prepared request bytes/digest, invocation settings, optional business key,
original plan/Contract and Retry policy. Later attempts reuse those values;
they do not re-evaluate invocation expressions or suffix the key with an
attempt ordinal. Secret references and binding attestations remain references,
never clear credentials copied into the journal. A preparation refusal does
not manufacture a Call or retryable terminal.

Attempts are one-based, from 1 through `Retry.attempts`. Each has a distinct
Call and at most one winning closure. The first uses the opening delivery.
Subsequent admissions preserve its DATA, travelling metadata and `origin_id`,
with a fresh temporal identity and the preceding terminal envelope as parent.
Their cursors are internal temporal admissions, not authored graph edges.
`Opened` and the first `Admitted` may share one atomic manager transaction.
Neither consumes the opening cursor; the first Connector completion owns
that consumption.

The first Call timeout includes its existing queue wait; opening the manager
does not reset it. Each later admission creates its own Call timeout. No Call
timeout runs during backoff.

### 3.1 Held and final batches

At the winning closure, the original policy and owner state decide whether
the selected terminal permits another attempt below the bound. A non-final
attempt's Connector transaction emits its complete ordered batch, including
frames and terminal, with an explicit manager disposition. `Held` records
exactly those emission ids; empty graph routes alone do not prove consumption.
No downstream Operator runs for that batch.

The manager later records `Consumed` and `Advanced` atomically. The next attempt
requires that consumption, its deadline if any, quiescence and capacity. At
most one unconsumed batch belongs to an operation. Consuming it releases pending
manager capacity, not journal history; the journal still retains every original
batch. Process `max_output` bounds raw stdout plus stderr, not decoded heap or
journal overhead.

The final attempt records `Published` and `Closed(Final)` in its Connector
transaction, using the original emissions and all authored destinations,
including `nothing`. Publication is not a second emission. Every selected
terminal can be final at the attempt bound, so `Retry.on` never waives
`E-UNWIRED-OUTPUT`. Failed capacity reservation commits neither publication
nor closure.

The common recurrence admission budget and dated scheduler serve both Loop and
Retry. A waiting operation still occupies capacity, but reserves no downstream
mailbox during backoff. Quiescence excludes its own pending batch and recurrence
deadline; it concerns this operation, not unrelated operations in the instance.
A containing Loop sees the entire Retry operation as outstanding until closure,
so one Retry does not consume additional Loop fuel per attempt.

### 3.2 Delay starts at closure

For failed attempt `i` starting at 1:

```text
fixed:       delay(i) = after
exponential: delay(i) = min(first * 2^(i - 1), ceiling)
due = saturating_add(winning_closed_at, delay(i))
```

Arithmetic is bounded before multiplication can overflow. The closure instant
is the explicit Execution/Lab logical time at which the winning closure is
accepted, not the Driver's physical timestamp. Every winning outbound Call
closure retains its instant, including response, timeout and abandonment,
whether or not Retry is present.

Only a selected non-final Retry closure creates the recurrence deadline, at
that original closure instant. No configured delay means no recurrence
deadline. A late manager or recovery never restarts the delay. A deadline in
the past makes admission eligible, not unconditional: consumption, quiescence,
capacity and the owner's lifecycle still apply. Equal-date order follows
recorded deadline creation order.

### 3.3 Closing

Closing prevents every further Retry admission, invalidates its wakeup and
never transfers work to a successor generation. During backoff the operation
closes with `Closed(Stopped)` and **no new graph output**. The held failure
remains in the journal under its original manager disposition.

If `Held` committed but `Consumed` did not, consumption and `Closed(Stopped)`
commit together, without `Advanced`. If already consumed, it is not consumed
twice. The same rule applies when a non-final winning closure preceded Closing
but its Connector transaction had not yet committed. Record order determines
the disposition; delayed projection cannot turn that held failure into a
public result.

A Call still open when Closing begins settles under ordinary cancellation,
response or abandonment rules. A final publication already committed is not
retracted. A request queued before Closing with **no opened Retry operation**
drains as one ordinary Call without Retry, including `single_pass`; its existing
queue timeout and Closing budgets remain. This exception does not rewrite an
already admitted Retry attempt.

## 4. Identity and trigger

[identity.md §3](identity.md#3-the-canonical-record-form), key **43**, owns the
Retry subform and fact tags. [envelope.md §2](envelope.md#2-identity--derived-zero-per-envelope-randomness) owns the
operation and admission identity recipes. This document allocates no tag,
record kind or separate journal.

The [trigger registry](envelope.md#31-transactions-without-a-port-arrival--trigger-and-parent)
and [execution-model.md §10](execution-model.md#10-trigger-vocabulary) give:

| Transaction | Trigger | Cause | Emitter |
|---|---|---|---|
| Retry manager transaction | `l:retry` | opening delivery's emitting id, or the recorded attempt terminal, else the operation's envelope | the Retry owner |

Connector transactions carrying `Held` or `Published` remain the original
producer's transactions. Correlation with Retry does not replace their trigger
or output identities.

## 5. Recovery and protocol facts

Recovery validates the original plan and Contract and reconstructs the exact
prefix. It checks owner generation, opening cursor, prepared material,
enclosing Loop contexts, ordered batches, dispositions and original deadlines.
It neither re-executes a handler nor recalls an old external effect. Later
attempts use the pinned invocation rather than re-evaluating it against their
new envelope ids.

| Fact | Recorded meaning |
|---|---|
| `Opened` | one exact opening and its prepared operation, before any effect |
| `Admitted` | one authorized request and attempt ordinal |
| `Held` | complete Connector batch explicitly destined for the manager |
| `Consumed` | acknowledgement of that exact batch, once |
| `Advanced` | consumed predecessor selects the following ordinal, atomically with `Consumed` |
| `Published` | final batch takes authored destinations, atomically with `Closed(Final)` |
| `Closed` | final publication or exact-generation stop ends the operation |

The ordinary EGRESS Call records carry opening and winning closure, correlated
to the attempt. After recovery:

| Last committed prefix | Remaining obligation |
|---|---|
| `Opened`, not `Admitted` | initial admission, using the same pinned operation |
| `Admitted`, no Call opening | authorized attempt may be opened by ordinary execution |
| Call opened, not closed | unresolved old Call; never start that effect again |
| Call closed, no Connector transaction | completion awaits its single emission transaction |
| `Held`, not `Consumed` | exact manager batch awaits consumption |
| `Consumed` + `Advanced`, not `Admitted` | next ordinal and original deadline, no fresh backoff |
| `Published` + `Closed` | resume ordinary committed routes/cursors, no second publication |

An unresolved old Call requires the existing timeout or `OutcomeUnknown`
settlement. Only its winning closure can authorize a distinct later attempt;
`idempotent = true` alone is not an instruction to recall it. Missing closure
instants, inconsistent deadlines, renewed delays and crossed batches refuse.
Diagnostic replay only reconstructs observations; it never executes pending
obligations. Durable journal storage remains outside M1.

## 6. Measurements and inspection

Typed response deserialization precedes Retry disposition, as specified by
[annotations.md §6.1](annotations.md#61-deserialization-supplies-the-values).
Every attempt's original typed measurements stay in its journaled emissions,
including held batches. Manager consumption and final routing create no
second sample. Missing quantities remain unknown, never a fabricated zero.
M1 provides no reporting engine, automatic pricing or subset aggregation;
[observability.md](observability.md) owns that scope and the privacy rules.

The debugger projects operation, attempt, held outcome, publication and due
time from the selected journal prefix. Retry contributes no spatial port,
command or edge to draw.

## 7. Named diagnostics

The [diagnostic registry](diagnostics.md) owns phases and payload fields.

| Code | Meaning |
|---|---|
| `E-RETRY-HOST` | owner or resolved Contract exchange cannot host Retry or Call idempotence |
| `E-RETRY` | missing, duplicated, misplaced or invalid policy attribute/bound/delay/guarantee; expression faults keep `E-EXPR` |
| `E-RETRY-CASES` | nonliteral, duplicate or unknown terminal selection, including predicates or quoted strings instead of case names |
| `E-RETRY-EFFECT` | selected outcome-unknown/after-effect case lacks the original plan's idempotence guarantee |

`E-RETRY-ATTRIBUTE` was retired on 23 October 2026 in favor of
`E-RETRY`; it is not an active diagnostic or alias. Incompatible same-owner
Extensions use `E-EXTENSION-CONFLICT`. Loop's advisory `W-LOOP-REPEAT` remains
distinct from Retry's hard admission rule.

## 8. Related specifications

- [loop.md](loop.md) — authored state recurrence and containing Loop obligations;
- [stdlib.md](stdlib.md) — exact Contracts, modes, terminal cases and visibility;
- [delivery-semantics.md](delivery-semantics.md) — commit before effect, dispositions and recovery;
- [instances.md](instances.md) — generation fences and Closing;
- [identity.md](identity.md) — canonical facts and unsealed deadline fields.
