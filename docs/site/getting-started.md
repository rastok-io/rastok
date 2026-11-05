---
title: Getting Started
description: Install the current Rastok CLI, create a typed graph with a Lab scenario, and inspect a simulation without external services.
order: 2
---

<!-- quickstart:start -->
Release binaries target macOS Apple silicon and Linux x86-64. Intel Macs and
other platforms build from source using the same command below.

Build from the repository with Rust **1.97.1** while release binaries are being
prepared. This installs HTTP/1.1, HTTP/2, WebSocket and the debugger, not HTTP/3.

```sh
cargo install --path crates/rastok --locked --features hosted-ui,hosted-websocket
rastok new hello
rastok simulate hello.rkg --ui
```

Open the loopback URL printed by the last command, including its access token.
The simulation starts paused. Choose **Next transition**, or **Finish** to
reach `Hello, world`. The CLI reports `PASS hello.roundtrip_greeting` when the scenario
finishes. Close the viewer from its toolbar when done.
<!-- quickstart:end -->

Debug: follow [The debugger, a guided tour](debugger.md) for its controls,
source view and assertion evidence.

Without the browser, `rastok simulate hello.rkg` runs the same scenario.
`rastok new` creates one file with a typed Operator and a Lab; it never
overwrites an existing file. Change the source on disk, then run it again.

Use `rastok --help` for the command syntax and `rastok --version` to identify
your build. The [Command line chapter](guide/09-command-line.md) explains all
commands, options, output streams and exit statuses.

From a checkout, continue with [echo](examples/01-echo.md) for a real Process
and an effect-free recorded replay. Then follow
[webhook to partner](examples/04-webhook-to-partner.md): its command supplies
the synthetic deployment profile and can open the same graph with `--ui`.
No real server, partner or credential is required for that simulation.
`rastok run` prints each App output value on its own line when the App declares
one output port. An App with several output ports prints `port: value` instead.
Downloadable binaries and two unaided timed installation trials are still required before
this quickstart is qualified for opening. No elapsed-time claim is made yet.

## Two ways to learn

Some people read the book first; others open an example and break it. Both
paths lead to the same place.

Keep [Reference](reference/types.md) open for types, expressions, Contracts,
Lab syntax and diagnostic codes while following either path.

- **By the book.** The [user guide](guide/01-a-program.md) reads the
  language in order: a program, types and expressions, Connectors and
  outcomes, the Lab, running for real, instances, recurrence, data policy,
  then the command line. Nine short chapters.
- **By example.** Eight examples, each built from a question, with a graph to
  read, a scenario to run, something to break and the first golden
  difference to read.

| Guide chapter | What it explains |
|---|---|
| [1. A program](guide/01-a-program.md) | Graphs, Nodes, ports and wiring |
| [2. Types and expressions](guide/02-types-and-expressions.md) | Values and typed decisions |
| [3. Connectors and outcomes](guide/03-connectors-and-outcomes.md) | External boundaries and explicit dispositions |
| [4. The Lab](guide/04-the-lab.md) | Scenarios, mocks and golden comparisons |
| [5. Running for real](guide/05-running-for-real.md) | Deployment and diagnostic replay |
| [6. Instances, state and time](guide/06-instances-state-and-time.md) | Ownership, lifetimes and bounded waits |
| [7. Loops and retries](guide/07-loops-and-retries.md) | Finite progression and safe repetition |
| [8. Privacy, secrets and measurements](guide/08-privacy-secrets-and-measurements.md) | Data policy and original quantities |
| [9. Command line](guide/09-command-line.md) | Commands, options and exit statuses |

| Example | Your question | What you learn to keep explicit |
|---|---|---|
| [1. Echo: one graph, two worlds](examples/01-echo.md) | Can I test a flow without starting its external program? | Computation, effects and the Lab |
| [2. File pipeline: effects with a contract](examples/02-file-pipeline.md) | What may this flow read or replace, and what happens when it cannot? | Bounded I/O and fault destinations |
| [3. Two tenants: whose state is this?](examples/03-two-tenants.md) | Can Alice retry while Bob is already done? | Instance ownership, finite retry and lifetime |
| [4. Webhook to partner: put it together](examples/04-webhook-to-partner.md) | Can I qualify an integration before the partner is available? | Admission, validation, deduplication, effects and audit |
| [5. Sensor alarm: make time an input](examples/05-sensor-alarm.md) | How do I test an absence, or a day of waiting? | Stateful decisions, timed joins and virtual time |
| [6. Agent gateway: keep a conversation isolated](examples/06-agent-gateway.md) | Can Alice and Bob share a graph without sharing a child process? | Resident Sessions, protocol adaptation and exact closure |
| [7. Countdown: a bounded loop](examples/07-countdown.md) | Can I prove that repeated work finishes? | Manager commands and a decreasing measure |
| [8. HTTP model adapter](examples/08-model-adapter.md) | What usage did the model actually report? | Typed responses, original measurements and absent versus zero |

Chapter 6 connects WebSockets to resident Process Sessions: Alice and Bob
keep distinct children and correctly addressed JSON replies. Chapter 8 sends
independent HTTP jobs and inspects typed usage supplied by the model. A
scripted answer exercises the adapter; it is not evidence of a real token bill.

## How to use the examples

Read in order, or take the short
route: [echo](examples/01-echo.md), then [webhook to partner](examples/04-webhook-to-partner.md).
Every chapter runs independently from the checkout; no chapter requires files
or state left by the previous one.

Run the scenario, inspect the graph, make the suggested edit on disk, and read
the first golden difference. Restore the edit before continuing. Keep a new
scenario when you discover a useful failure: it becomes a check for your next
revision. The debugger explains recorded execution; it does not edit the flow
or fork an alternative world.

By the end, the same ideas apply to a file, a partner API and an agent:
who owns the state, where effects occur, what time means, and where each outcome
goes. You do not need a different testing model for each integration.

## After the public opening

The planned 5 November 2026 download location is
[Rastok releases](https://github.com/rastok-io/rastok/releases): macOS Apple
silicon and Linux x86-64, with `hosted-ui,hosted-websocket`. Intel Macs and
other platforms use the source-install fallback. Once the
repository and the `v0.1.0-alpha` tag are published,
the Rust 1.97.1 source-install fallback is:

```sh
cargo install --git https://github.com/rastok-io/rastok --tag v0.1.0-alpha --locked --features hosted-ui,hosted-websocket rastok
```

This command was qualified against the public `v0.1.0-alpha` tag on
5 November 2026 (installation from a clean Cargo home, `rastok --version`,
`rastok simulate examples/echo.rkg`). The public documentation address is
[docs.rastok.com](https://docs.rastok.com).
