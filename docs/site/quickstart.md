---
title: Quickstart
description: Install the current Rastok CLI, create a typed graph with a Lab scenario, and inspect a simulation without external services.
order: 2
---

<!-- quickstart:start -->
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
