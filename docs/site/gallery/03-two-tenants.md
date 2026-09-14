---
title: Two tenants — whose state is this?
description: Watch Bob finish once while Alice alone times out, waits for backoff and retries in an isolated keyed instance.
teaches: Keyed instances, idle lifetime, nested Graph, bounded Loop, explicit retry state
graph: examples/keyed-retry.rkg
golden: crates/rastok/tests/golden/gallery/two-tenants/ui_tour.two_tenants.trace
command: rastok simulate examples/keyed-retry.rkg --golden crates/rastok/tests/golden/gallery/two-tenants
expected: Two scenarios pass; Bob completes in one attempt, Alice in two, and idle expiry opens a fresh generation.
break_it: 'Change Loop.fuel = 2 to Loop.fuel = 1, then rerun without --bless.'
limits:
  - An instance key isolates state; it does not deduplicate requests.
  - The Lab substitutes Process responses; no model or real agent is called.
  - Idle lifetime and Loop iterations are different counters; restart durability is not provided.
replaces: 'NiFi: keyed processing plus RetryFlowFile and a nested process group; MuleSoft: scoped processing with bounded retry.'
order: 3
---

A file pipeline is easy to follow one request at a time. What happens when
Alice's partner stalls while Bob's request is ready? This chapter adds
instance ownership and finite retry to the same graph-and-Lab model.

## Run

```sh
rastok simulate examples/keyed-retry.rkg --golden crates/rastok/tests/golden/gallery/two-tenants
```

Add `--ui` to follow the same checked scenario in the debugger.

## Read the graph

Start with `instance = { key = data.tenant }`: it selects independent state
for Alice and Bob. `Loop.fuel` bounds the applications within one run;
`lifetime` bounds how long an idle instance lives. They solve different problems.
Then follow `model.exit` into `decide`: its typed match chooses a business
answer, failure, or the `Loop.next` command with explicit retry state.
The nested `audit` Graph adds composition, not another execution model.

## Expected result

Alice enters first and waits. Bob enters a separate instance and finishes
immediately, with `attempts = 1`. Advancing one second expires Alice's Call;
the next second admits her retry, which finishes with `attempts = 2`. Bob
receives no second Call. The `lifetime` scenario advances through ten seconds
of silence, then proves that a new admission uses a fresh instance generation.

`⧉` counts instances; `⟳` describes the selected Loop's admitted applications.
The idle lifetime does not reload fuel during a run. The nested audit Graph
only wires its contained Operator; it executes no hidden handler.

## Break it

Change `Loop.fuel = 2` to `Loop.fuel = 1`. Alice can no longer be readmitted:
`LoopExpired` reaches the existing `failed` terminal. Bob still finishes once.

## Read the golden difference

The golden names the first changed Loop record and its retained context.
It may diverge at the opening fuel fact, before the final missing answer.
The failed answer assertion is a consequence; the CLI does not guess a cause
beyond retained facts. Restore the fuel before continuing.

## Carry forward

Ask two separate questions in every stateful flow: "whose state?" and "for
how long?" A key answers the first, a lifetime the second. Neither alone
means that a duplicate request will be ignored. The next chapter will make
that policy an explicit piece of state.

Previous: [File pipeline](02-file-pipeline.md). Next:
[Webhook to partner](04-webhook-to-partner.md), combining these mechanisms
into an integration you can qualify before its partner is ready.

## Limits

The Process mock is stateless: the explicit `attempt` field selects the reply.
No real language model or paid API is used. The instance key isolates state but
does not guarantee deduplication or persistence across a deployment restart.
The fixed backoff is one second; this example does not benchmark model latency.

## NiFi / MuleSoft equivalent

Keyed processing, a nested process group and a bounded retry path. The visual
debugger exposes the state isolation and temporal recurrence; Loop commands
are not spatial back-edges.
