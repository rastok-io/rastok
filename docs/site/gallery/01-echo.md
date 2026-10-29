---
title: Echo — one graph, two worlds
description: Run one graph with a real cat process or scripted Lab responses, then inspect a recording without starting the process again.
teaches: Operator, Connector, Graph, Lab, causal golden, diagnostic replay
graph: examples/echo.rkg
golden: crates/rastok/tests/golden/gallery/echo/smoke.roundtrip.trace
command: rastok simulate examples/echo.rkg --compare crates/rastok/tests/golden/gallery/echo
expected: Three scenarios pass, including a filtered empty line and a timed-out silent process.
break_it: 'Change the mock line out = . to out = "changed", then rerun the command without --save.'
limits:
  - Equal causal facts require equal inputs, responses and observable logical time.
  - Diagnostic replay reconstructs recorded facts; it does not execute a Lab scenario.
  - Recordings are sensitive local files, not a durable deployment journal or share-safe exports.
order: 1
---

Can you test a flow before its external program is available? Start with one
line of text. This first chapter separates what your program decides from
what the outside world does, then keeps both success and silence as scenarios.

## Run

From the checkout, with `rastok` installed:

```sh
rastok simulate examples/echo.rkg --compare crates/rastok/tests/golden/gallery/echo
```

## Read the graph

Read the App's wiring, then its Operator and `shell` Connector. The Operator
filters and forwards data; `Process` is the explicit external effect. Finally,
read the `lab` block: its mock supplies the world's response while the
application graph stays the same. The Graph composes these parts; it does not
hide an extra handler.

## Expected result

Three scenarios pass. A nonempty line is echoed, an empty line never calls the
Process, and a silent Process produces `TimeoutExpired` after virtual time
advances. Simulation uses the Lab; it never starts `cat`.

For the same `salut` input used by the roundtrip scenario:

```sh
printf 'salut\n' | rastok run examples/echo.rkg --record echo.rkrec
rastok replay echo.rkrec
rastok replay echo.rkrec --ui
```

The real run and recorded replay print `salut` and the same causal seal.
Only the first command starts `cat`. The recording viewer needs neither the
source nor the original process. Use a new filename for another recording.
Comparing a real run with a Lab execution additionally requires equal external
responses and observable logical time; a recording is not automatic Lab extraction.
The standalone commands also retain different ingress provenance (`cli` versus
`lab`). Their raw seals therefore **do not match by default**, even for the same
displayed text. This page proves run-to-recorded-replay equality and lets you
compare the Lab's output separately; it does not erase provenance to force a match.

## Break it

In `mock shell`, change `out = .` to `out = "changed"`. Rerun the first
command **without** `--save`. The output assertion and golden should fail.

## Read the golden difference

The diagnostic names the first differing record and field, labels the expected
and actual values, and reports the recorded input/transaction/Call context.
Later differences are consequences, not a guessed source-code cause.
Restore the line and rerun; do not approve a new golden merely to make a failure disappear.

## Carry forward

You now have a small behavior you can run, explain and change under a test.
A failure is an input to the Lab, not a special copy of your application.
Keep that distinction: it is how you can start integration work before a
partner environment exists.

Next: [File pipeline](02-file-pipeline.md). Replace the external program with
two file operations and ask what each effect is allowed to do.

## Limits

`run` requires `cat` on PATH. Golden files contain causal evidence, not a
digital signature. Timings measured by a real process need not match virtual
time; only specified causal fields participate in the seal. Recordings are
bounded, local diagnostics and currently refuse personal-data persistence and
boot-dependent deployment profiles.
