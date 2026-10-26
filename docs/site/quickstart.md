---
title: Quickstart
description: Install the current Rastok CLI, create a typed graph with a Lab scenario, and inspect a simulation without external services.
order: 2
---

<!-- quickstart:start -->
Release binaries target macOS Apple silicon and Linux x86-64. Intel Macs and
other platforms build from source using the same command below.

Build from the repository with Rust **1.97.1** while release binaries are being
prepared. This installs HTTP/1.1–2, WebSocket and the debugger, not HTTP/3.

```sh
cargo install --path crates/rastok --locked --features hosted-ui,hosted-websocket
rastok new hello
rastok simulate hello.rkg --ui
```

Open the loopback URL printed by the last command, including its access token.
The simulation starts paused. Choose **Next transition**, or **Finish** to
reach `Hello, world`. The CLI reports `PASS smoke.roundtrip` when the scenario
finishes. Close the viewer from its toolbar when done.
<!-- quickstart:end -->

Without the browser, `rastok simulate hello.rkg` runs the same scenario.
`rastok new` creates one file with a typed Operator and a Lab; it never
overwrites an existing file. Change the source on disk, then run it again.

From a checkout, continue with [echo](gallery/01-echo.md) for a real Process
and an effect-free recorded replay. Then follow
[webhook to partner](gallery/04-webhook-to-partner.md): its command supplies
the synthetic deployment profile and can open the same graph with `--ui`.
No real server, partner or credential is required for that simulation.
Downloadable binaries and two unaided timed installation trials are still required before
this quickstart is qualified for opening. No elapsed-time claim is made yet.

## After the public opening

The planned 5 November 2026 download location is
[Rastok releases](https://github.com/rastok-io/rastok/releases): macOS Apple
silicon and Linux x86-64, with `hosted-ui,hosted-websocket`. Intel Macs and
other platforms use the source-install fallback. Once the
repository and `v0.1.0-alpha` tag have been published on Yoann's instruction,
the Rust 1.97.1 source-install fallback is:

```sh
cargo install --git https://github.com/rastok-io/rastok --tag v0.1.0-alpha --locked --features hosted-ui,hosted-websocket rastok
```

This command is prepared for opening, not yet qualified against a public tag.
Until then, use the checkout installation above. The public documentation
address is [docs.rastok.com](https://docs.rastok.com).
