---
title: 'Countdown: a bounded loop'
description: Prove that a countdown terminates, follow its manager commands, and see the checker reject a measure that cannot prove progress.
teaches: Loop.next, Loop.state, decreasing measures, finite recurrence and the host terminal
graph: examples/countdown.rkg
golden: examples/traces/countdown/proof.positive.trace
command: rastok simulate examples/countdown.rkg --compare examples/traces/countdown
expected: Two scenarios pass; input 3 reaches liftoff after three recurrences and input 0 finishes immediately.
break_it: 'Replace Loop.measure = Loop.state.data with Loop.measure = 3 and run rastok check examples/countdown.rkg; E-LOOP refuses the unsupported measure.'
limits:
  - The direct measure proof accepts an integer path, a positivity guard and a literal strict decrease, not arbitrary termination arguments.
  - This measure-only example proves exhaustion impossible and therefore declares no LoopExpired output.
order: 7
---

This program counts down from an integer and emits one final message.

A conversation keeps a resource alive until closure. A countdown asks the
opposite question: can the program prove that repeated work will finish?
The answer is written beside the Operator, without a backwards graph edge.

The [file formats](../guide/09-command-line.md#file-formats) are `.rkg` for
the program and Lab, `.trace` for a reviewed scenario reference and `.rkrec`
for a real execution recording.

## Run

From the repository root, with `rastok` installed:

```sh
rastok simulate examples/countdown.rkg --compare examples/traces/countdown
```

```text
PASS proof.positive: three recurrences, then one result
PASS proof.already_finished: a nonpositive input needs no recurrence
2 scenario(s), 0 failed
```

Add `--ui` to inspect each admitted application and its manager transition.

```sh
rastok simulate examples/countdown.rkg --compare examples/traces/countdown --ui
```

This requires `hosted-ui`. Open the private loopback URL printed by the CLI.

For the controls and source view, see [the debugger tour](../debugger.md).

## Read the program

The App wires `in -> count -> out`. Inside `count`, a positive integer produces
`Loop.next = data - 1`; otherwise the Operator emits `liftoff`. `Loop.next`
is a command to the Loop manager, not a port or a graph edge. The manager
records the command and admits the next application after the previous one
has finished.

`Loop.state` is the envelope admitted for the current application.
`Loop.measure = Loop.state.data` selects its integer DATA as the decreasing
quantity. The positivity guard and subtraction by one provide the proof.
The terminal `out` supplies the final Done verdict.

This example needs no exhaustion output because its measure proves termination.
With an explicit fuel bound, possible exhaustion instead produces
`LoopExpired { last_state: EntryType }` on the host's declared terminal,
normally `err` for an Operator or `exit` for a Call. It must have a destination.
Loop never adds a synthetic expiry port. See `docs/spec/loop.md` §§8 and 10.

There is no Connector in this program. The [Lab reference](../reference/lab.md)
explains expectations and observation windows; `advance` supplies virtual time.

## Expected result

`proof.positive` admits states 3, 2, 1 and 0, then emits one `liftoff`.
`proof.already_finished` admits 0 and emits the same result without recurrence.
The CLI ends with `2 scenario(s), 0 failed`.

Inspect the positive trace: ordinary Operator transactions produce commands;
Manager transactions, labeled `l:loop` in the trace, record its progress. Four applications and
three recurrences are different counts. Each application has its own causal
identity even when it belongs to the same run.

## Break it

Replace `Loop.measure = Loop.state.data` with `Loop.measure = 3`, then run:

```sh
rastok check examples/countdown.rkg
```

The checker reports exactly:

```text
error[E-LOOP]: this Loop cannot be proved finite: the Loop measure is outside the supported direct integer-path fragment
```

Restore the measure before trying the next edit (this discards edits to the example):

```sh
git checkout -- examples/countdown.rkg
```

For a runtime comparison, change only the Operator's
`out = "liftoff"` to `out = "changed"` and run the complete comparison below.

```sh
rastok simulate examples/countdown.rkg --compare examples/traces/countdown
```

```text
FAIL proof.positive: three recurrences, then one result
     expect app.out count == 1 where data == "liftoff"  (countdown.rkg:25)
     observed app.out: "changed"
     where data == "liftoff" is false for 1 of 1 deliveries; count 0, expected 1
     golden divergence: record 14: emissions(23)[0][2] . data(5)[1] (ccbor value)[1]; expected "liftoff"; got "changed"; cause: TRANSACTION "count" · trigger "p:in" · cause_id absent
FAIL proof.already_finished: a nonpositive input needs no recurrence
     expect app.out count == 1 where data == "liftoff"  (countdown.rkg:29)
     observed app.out: "changed"
     where data == "liftoff" is false for 1 of 1 deliveries; count 0, expected 1
     golden divergence: record 5: emissions(23)[0][2] . data(5)[1] (ccbor value)[1]; expected "liftoff"; got "changed"; cause: TRANSACTION "count" · trigger "p:in" · cause_id absent
2 scenario(s), 2 failed
```

Restore the checked-in source (this discards your edits to this example):

```sh
git checkout -- examples/countdown.rkg
```

## Read the golden difference

The invalid measure is rejected before execution, so it creates no new trace.
The changed output, by contrast, changes an emission: the golden comparison
names the first differing record, expected and actual values, and its cause.
The unchanged scenario expectation also rejects the changed text. Restore the
output rather than saving over the reference.

## Carry forward

Previous: [Agent gateway](06-agent-gateway.md). Separate a resource's lifetime
from a finite algorithm's progress. A Loop advances authored state; a Retry
repeats a prepared operation. Next: [HTTP model adapter](08-model-adapter.md),
which adds typed usage evidence to an external response.

## Limits

This is a pure integer countdown, with no external program or elapsed-time
claim. The direct proof fragment is deliberately small; unsupported proofs
require a finite fuel bound and an explicit exhaustion disposition.
The journal remains in memory; a golden trace is comparison evidence, not
durable recovery.
