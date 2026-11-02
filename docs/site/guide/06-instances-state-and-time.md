---
title: Instances, state and time
description: Choose the owner of state, separate instance keys from lifetimes, and make closure and bounded waiting explicit.
order: 6
---

After this chapter you can choose an instance key, distinguish immutable
values from private mutable state, and explain when a lifetime or a bounded
join produces work. The central question is ownership: which tenant or sensor
does this arrival belong to, and which live instance may consume it?

## Select an instance

In `examples/keyed-retry.rkg`, lines 29–31, the App declares three independent
policies:

```rkg
instance = { key = data.tenant }
lifetime = { kind = idle; after = duration("10s") }
closing = { drain_for = duration("1s"); settle_for = duration("1s") }
```

The key selects Alice's or Bob's App instance before delivery. Its descendants
belong to that selected parent instance: an unkeyed child is a singleton
inside each parent, not a global singleton shared by both tenants. Each
Node has an instance: an Operator owns its mailbox and state; a Connector
owns its open exchanges. A Graph owns
containment and lifecycle, not a shared application transaction.

The key reads the entering envelope and permitted enclosing immutable
values. Its type must be valid on every admitting input. Changing the payload (DATA) later
does not move the current instance. When an address closes and later opens
again, a new generation separates its clean state from old deliveries and
deadlines. The journal records that exact destination generation.

## Values and mutable state

A `val` is immutable in its lexical scope. A handler-local value is computed
for that transaction; a Node-level value is derived for the instance and may
be used by descendants. It is useful for shared constants and settings, not
for a counter that changes on each arrival.

A `var` belongs to one Operator instance. In `examples/sensor-alarm.rkg`,
lines 19–20, the key and the mutable cell are adjacent:

```rkg
instance = { key = data.sensor }
var tripped: bool = false
```

Pump-a can be tripped while pump-b is normal. The state change and emission
commit together in the owning Operator transaction. Another Operator cannot
read that cell by name; shared mutable behavior needs an explicit Operator
with typed inputs and outputs. See `docs/spec/instances.md` §§3–5.

## Choose a lifetime

Lifetime is independent of the key. The attribute has the shape
`lifetime = { kind = ... }`, with `after` or `every` where required.

| Kind | Meaning and additional field |
|---|---|
| `persistent` | No lifetime deadline; this is also the omitted policy |
| `idle` | Close after admission silence, with `after = duration("10s")` |
| `fixed` | Close at a fixed age from the creating admission, using `after` |
| `single_pass` | Admit one entering envelope into a generation, then request Closing |
| `tumbling` | Select the epoch-aligned processing-time box at admission, with `every = duration("5m")` |

Traffic refreshes an idle deadline, not a fixed one. An open Call alone is
not admission activity. Tumbling boundaries are independent of boot time;
the run's recorded start timestamp maps them onto the logical timeline.

## Close with explicit budgets

The example's `closing` policy first permits admitted work to drain, then
allows canceled Calls to settle. Both durations are positive. A confirmed
terminal outcome stays its normal typed exit; an unconfirmed outcome at the
settlement deadline becomes `OutcomeUnknown`.

A finite owner containing Process requires this policy. Children close
before their parent, and later arrivals wait for a successor generation.
Closure is neither a rollback nor permission to drop the old generation's
outcomes. The full sequence is in `docs/spec/instances.md` §7.1.

## Bound a wait, then advance time

`examples/sensor-alarm.rkg`, lines 33–42, correlates within the sensor instance:

```rkg
operator watchdog {
  input probe: Sensor
  input heartbeat: Sensor
  output out: text
  output expired: MissedHeartbeat
  instance = { key = data.sensor }
  on (probe, heartbeat) within duration("5m") {
    out = probe.data.sensor
  }
}
```

`within` bounds how long each candidate waits; `last N` instead bounds the
number retained per input. A bounded join must declare and wire `JoinExpired`
on its `expired` output, even if the intended scenario always supplies both
inputs. Deliberate `-> nothing` is a destination; silence is not.

The Lab's `advance` visits due deadlines without sleeping. A partial join
can be waiting while no work is runnable. `now()` observes the transaction's
civil timestamp, derived from origin plus logical time; it does not trigger
a handler. See `docs/spec/execution-model.md` §§5, 7 and 8.

## Summary

Keys select owners, `var` stores private state, and lifetimes determine when
instances close. Joins bound retained candidates; the Lab advances their
deadlines explicitly. Sliding/event-time windows and durable recovery are
outside the [current alpha scope](../index.md#what-is-available-today).

## Next

Continue with [Loops and retries](07-loops-and-retries.md), then compare [Two tenants](../examples/03-two-tenants.md)
and [Sensor alarm](../examples/05-sensor-alarm.md). Explain which instance owns
each pending Call or unmatched probe before running the scenario.
