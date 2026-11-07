---
title: Getting Started
description: Install the current Rastok CLI, create a typed graph with a Lab scenario, and inspect a simulation without external services.
order: 2
---

## Install and run Rastok

<!-- quickstart:start -->
Rastok is currently in alpha. Two prebuilt binaries are available:
**macOS Apple silicon** and **Linux x86-64**, on the
[release page](https://github.com/rastok-io/rastok/releases/tag/v0.1.0-alpha).
Copy the command for your system below into a terminal, in an empty directory.
It downloads, verifies and installs Rastok.

### macOS Apple silicon

For Macs with an Apple M-series chip, running macOS 11 or later:

```sh
curl -fLO https://github.com/rastok-io/rastok/releases/download/v0.1.0-alpha/rastok-0.1.0-alpha-aarch64-apple-darwin.tar.gz && curl -fLO https://github.com/rastok-io/rastok/releases/download/v0.1.0-alpha/rastok-0.1.0-alpha-aarch64-apple-darwin.tar.gz.sha256 && shasum -a 256 -c rastok-0.1.0-alpha-aarch64-apple-darwin.tar.gz.sha256 && tar -xzf rastok-0.1.0-alpha-aarch64-apple-darwin.tar.gz rastok && mkdir -p "$HOME/.local/bin" && install -m 755 rastok "$HOME/.local/bin/rastok" && export PATH="$HOME/.local/bin:$PATH" && rastok --version
```

### Linux x86-64

For 64-bit Intel or AMD systems with glibc 2.39 or later, such as Ubuntu 24.04.
`uname -m` should report `x86_64`; `ldd --version` reports your glibc version.
For Linux ARM64, older glibc or a system using musl, use the source installation
below.

```sh
curl -fLO https://github.com/rastok-io/rastok/releases/download/v0.1.0-alpha/rastok-0.1.0-alpha-x86_64-unknown-linux-gnu.tar.gz && curl -fLO https://github.com/rastok-io/rastok/releases/download/v0.1.0-alpha/rastok-0.1.0-alpha-x86_64-unknown-linux-gnu.tar.gz.sha256 && sha256sum -c rastok-0.1.0-alpha-x86_64-unknown-linux-gnu.tar.gz.sha256 && tar -xzf rastok-0.1.0-alpha-x86_64-unknown-linux-gnu.tar.gz rastok && mkdir -p "$HOME/.local/bin" && install -m 755 rastok "$HOME/.local/bin/rastok" && export PATH="$HOME/.local/bin:$PATH" && rastok --version
```

Each command installs `rastok` in `~/.local/bin` for your user account and
prints `rastok 0.1.0-alpha`. To use it in new terminals, add
`export PATH="$HOME/.local/bin:$PATH"` to your shell configuration:
`~/.zshrc` for zsh (the macOS default), or `~/.bashrc` for bash.

### Other platforms and source builds

<details>
<summary>Installation and contribution instructions</summary>

Rastok is written in Rust. You can compile it from source if your platform has
no prebuilt binary, or if you prefer to build it yourself.

First, follow the [official Rust installation instructions](https://rust-lang.org/tools/install/),
including the build tools listed for your system. This also installs `rustup`,
which selects the Rust version, and Cargo, which builds and installs Rust
software. Open a new terminal and install the version used by Rastok:

```sh
rustup toolchain install 1.97.1
```

**Install the published release.** Cargo downloads the sources and dependencies,
compiles them on your machine and installs the executable:

```sh
cargo +1.97.1 install rastok --version 0.1.0-alpha --locked --features hosted-ui,hosted-websocket
rastok --version
```

**Modify the source and contribute.** Clone the repository instead, then build
and run your local copy. For a pull request, fork the repository on GitHub
first and use your fork's URL in the clone command:

```sh
git clone https://github.com/rastok-io/rastok.git
cd rastok
cargo +1.97.1 run --locked -p rastok --features hosted-ui,hosted-websocket -- --help
```

Cargo builds and runs the executable from your working copy. After editing,
repeat this command or replace `--help` with the Rastok command you want to test.
The [contribution guide](https://github.com/rastok-io/rastok/blob/main/CONTRIBUTING.md)
explains the checks and review process for a pull request.

Both installation routes include HTTP/1.1, HTTP/2, WebSocket and the debugger;
HTTP/3 is not included.

</details>

### Create your first application

In a directory for your work, run:

```sh
rastok new hello
rastok simulate hello.rkg --ui
```

Open the loopback URL printed by the last command, including its access token.
The simulation starts paused. Choose **Next transition**, or **Finish** to
reach `Hello, world`. The CLI reports `PASS hello.roundtrip_greeting` when the scenario
finishes. Close the viewer from its toolbar when done.
<!-- quickstart:end -->

## The file `rastok new hello` writes

The whole program, nineteen lines: one App graph with typed ports, one
Operator with a handler, and a Lab with one scenario. No Connector, no
external system.

```rkg
graph hello : App {
  input in: text
  output out: text

  operator greet {
    input in: text
    output out: text
    on in { out = "Hello, " + data }
  }

  in -> greet -> out
}

lab hello {
  scenario roundtrip_greeting {
    app.in = "world"
    expect app.out == "Hello, world"
  }
}
```

`rastok check hello.rkg` answers `ok: 2 node(s)` with the semantic hash;
`rastok simulate hello.rkg` prints `PASS hello.roundtrip_greeting` and
`1 scenario(s), 0 failed`. Start from this file rather than from the grammar;
[Echo](examples/01-echo.md) then adds a Connector, a mock and a recording.

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
Two unaided timed installation trials are still required before this
quickstart is qualified. No elapsed-time claim is made yet.

## Three ways to learn

Some people read the book first; others open an example and break it; others
hand the first step to a coding assistant and read what it wrote. All three
paths lead to the same place.

Keep [Reference](reference/types.md) open for types, expressions, Contracts,
Lab syntax and diagnostic codes while following any of them.

- **By the book.** The [user guide](guide/01-a-program.md) reads the
  language in order: a program, types and expressions, Connectors and
  outcomes, the Lab, running for real, instances, recurrence, data policy,
  then the command line. Nine short chapters.
- **By example.** Eight examples, each built from a question, with a graph to
  read, a scenario to run, something to break and the first golden
  difference to read.
- **With a coding assistant.** Open your preferred LLM harness or coding
  agent in an empty directory and give it a specification, for example:
  *"Using https://docs.rastok.com/llms.txt, develop a Rastok application and
  its Lab that meet the following specification: \<your spec\>."* The
  documentation tells it how to check and simulate what it wrote; read the
  result as you would read a colleague's: the compiler and the scenarios are
  the review.

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
