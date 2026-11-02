---
title: 'Loops and retries: bounded recurrence'
description: Separate authored state progression from safe repetition of one prepared Call, with explicit bounds, delays and terminal outcomes.
order: 7
---

After this chapter you can choose Loop for state progression and Retry for
repeating one operation, explain their bounds, and identify the guarantee
needed before repeating a visible external effect. Both use a journal-owned
manager. Neither introduces a backwards edge into the directed acyclic graph (DAG).

## Advance a state with Loop

Read `examples/countdown.rkg`, lines 7–16:

```rkg
operator count : Loop {
  input in: int
  output out: text
  Loop.measure = Loop.state.data

  on in match {
    data > 0 => Loop.next = data - 1
    _        => out = "liftoff"
  }
}
```

Each application either produces a new state through `Loop.next` or ends
through its terminal output. The command is recorded in the producing
transaction. The manager later consumes it and admits another application.
It is a command destination, not an output port or an edge to wire backwards.

`Loop.state` is the complete envelope admitted for the current application.
Its DATA is an integer here. `Loop.measure` selects the quantity whose strict
decrease proves termination: a direct integer path, a positivity guard and
literal subtraction are within the executable proof fragment. Input 3 needs
four applications, ending at 0. A constant measure is not a progress proof.

`Loop.again` retains that admitted state; `Loop.done` finishes without a
business output. Every application needs exactly one verdict. A terminal
output such as `out` supplies Done implicitly; `nothing` only disposes an
output and does not itself finish a Loop application.

## Bound and delay the applications

At least one of `Loop.measure` or `Loop.fuel` is required. Positive
`Loop.fuel = N` permits at most N applications, including the first. If the
last application requests another, the manager emits
`LoopExpired { last_state: EntryType }` on the existing host terminal. An
Operator normally declares `err` for that case; a Call uses `exit`. The
terminal must be typed and wired like every other productible outcome.
The measure-only countdown proves exhaustion impossible, so it needs no
expiry declaration.

`Loop.delay` may be fixed, with `kind = fixed` and `after`, or exponential,
with `kind = exponential`, `first` and `ceiling`. Durations are positive;
there is no jitter. The delay starts when the manager consumes the command
after the application has no outstanding work. No next application overlaps
the previous one. See `docs/spec/loop.md` §§7–10.

## Repeat an operation with Retry

The webhook uses `examples/webhook-to-partner.rkg`, lines 101–105:

```rkg
idempotent = true
behavior = deterministic
Retry.attempts = 2
Retry.on = [ServerError, TimeoutExpired, DnsFailed, ConnectFailed]
Retry.delay = { kind = fixed; after = duration("1s") }
```

The manager retains one prepared request, including evaluated settings and
the partner key. Attempts reuse it; invocation expressions are not evaluated
again. `Retry.attempts` includes the first Call. `Retry.on` is a static set
of terminal case names, not a predicate over arbitrary response DATA.

Fixed delay uses `after`. Exponential delay uses `first` and `ceiling`, with
`first <= ceiling`; after failed attempt i its delay is
`min(first * 2^(i - 1), ceiling)`. It starts at the winning Call closure,
not when a delayed manager happens to run. Each attempt remains in the
journal; only the final batch, its response frames and terminal outcome,
is delivered downstream. Exhaustion forwards
the last terminal unchanged, without a new Retry error case.

## Check repetition safety separately

The Contract classifies every terminal:

| Class | Meaning for repetition |
|---|---|
| `before_effect` | The operation has not become externally visible; Retry may select it without an idempotence guarantee |
| `outcome_unknown` | Visibility or outcome is uncertain; Retry requires `idempotent = true` |
| `after_effect` | The operation became visible; Retry requires `idempotent = true` |

The checker enforces that rule for every selected case. A decoded HTTP 500
is `ServerError`, an `after_effect` response, not a transport fault. The
webhook's real partner must honor the declared same-request guarantee; merely
sending an idempotency key does not establish it.

`behavior = deterministic | probabilistic` describes results, not repetition
safety. A deterministic payment can still charge twice. An inference may
return different text and incur another charge, which is why the model
adapter omits Retry. Loop proves finitude, not effect safety; its advisory
`W-LOOP-REPEAT` is distinct from Retry's hard refusal.

## Summary

Loop chooses the next state; Retry keeps the same prepared operation.
Their bounds and delays are explicit. A Loop Graph may contain Retry Calls;
the enclosing application waits for them through backoff and final closure.
Compensation belongs to the future Saga contract, not a global `effect`
attribute. See `docs/spec/retry.md`, `docs/spec/effects-and-sagas.md` and the
[current alpha scope](../index.md#what-is-available-today).

## Next

Continue with [Privacy, secrets and measurements](08-privacy-secrets-and-measurements.md). Practice the distinction
in [Countdown](../examples/07-countdown.md) and
[Webhook to partner](../examples/04-webhook-to-partner.md): identify the
verdict or terminal that permits each recurrence before advancing time.
