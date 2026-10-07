---
title: Learn Rastok by building integrations
description: Follow a hands-on path from one echoed message to isolated retries, a partner integration and virtual time, keeping failures as executable scenarios.
order: 1
---

An integration rarely arrives with all its partners, test data and failure
environments ready. With Rastok, you write the flow and its possible worlds
together. The same checked graph runs against scripted Lab outcomes or real
Drivers. Its behavior becomes something you can version, inspect and test
again when the next change arrives.

This is a short, hands-on book. Start with one message, then add one difficulty
at a time. Each chapter asks a question, runs an example, and invites you to
break it. The point is to understand why it behaved that way, not just to
collect green tests.

## Learning path

| Chapter | Your question | What you learn to keep explicit |
|---|---|---|
| [1. Echo — one graph, two worlds](gallery/01-echo.md) | Can I test a flow without starting its external program? | Computation, effects and the Lab |
| [2. File pipeline — effects with a contract](gallery/02-file-pipeline.md) | What may this flow read or replace, and what happens when it cannot? | Bounded I/O and failure destinations |
| [3. Two tenants — whose state is this?](gallery/03-two-tenants.md) | Can Alice retry while Bob is already done? | Instance ownership, finite retry and lifetime |
| [4. Webhook to partner — put it together](gallery/04-webhook-to-partner.md) | Can I qualify an integration before the partner is available? | Admission, validation, deduplication, effects and audit |
| [5. Sensor alarm — make time an input](gallery/05-sensor-alarm.md) | How do I test an absence, or a day of waiting? | Stateful decisions, timed joins and virtual time |
| [6. Agent gateway — keep a conversation isolated](gallery/06-agent-gateway.md) | Can Alice and Bob share a graph without sharing a child process? | Resident Sessions, protocol adaptation and exact closure |

Chapter 6 now composes WebSockets with resident Process Sessions: Alice and
Bob retain distinct children and correctly addressed JSON replies. A smaller
dialogue primer remains available. Public authentication and the HTTP variant
with model credentials and measured usage are still separate open work;
neither a socket write nor a scripted answer is evidence of a token bill.

## How to use this book

Install with the [quickstart](quickstart.md). Read in order, or take the short
route: [echo](gallery/01-echo.md), then [webhook to partner](gallery/04-webhook-to-partner.md).
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

## What is available today

Rastok is experimental. The deployment journal is in memory; durable storage
is later work. Diagnostic replay restores recorded facts, not the world.
The six chapters are executable; the complete AI gateway and the full
opening qualification remain in progress. No reading-time claim is made before
independent quickstart trials.
