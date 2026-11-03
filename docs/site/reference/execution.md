---
title: Execution and concurrency
description: Distinguish instance ownership, concurrent external I/O and the execution model implemented by the current runtime.
order: 7
---

## Current runtime

The hosted runtime advances a program through one single-threaded Worker.
It executes Operator transactions sequentially, not in parallel on several
CPU cores. A Driver can have an external Call outstanding while the Worker
processes other ready work. Network I/O and child processes can therefore
overlap without parallel execution of DSL handlers.

A transaction prepares its state changes and emissions, then commits them
together. It does not wait for a partner response inside an Operator
handler. A Connector's later response or timeout supplies another activation.
A long-running Operator computation can delay other ready work; the alpha
does not schedule it on a separate compute pool.

## Instances own state, not threads

| Concept | Current meaning |
|---|---|
| Instance key | Selects a logical owner, such as one tenant or sensor; it does not allocate a thread or process |
| Operator instance | Owns its mailbox and private `var` cells; its transactions run one at a time |
| Connector instance | Owns its exchanges and Contract state; a Driver performs the external operations |
| Graph instance | Owns containment and lifecycle, but has no application mailbox, handler or shared mutable state |
| Lifetime | Determines when an instance closes; it is independent of key selection and physical placement |
| Worker | Advances the execution, journal and selected Drivers; it is not created for each key |

An unkeyed child belongs to its enclosing Graph instance. Keying the App by
tenant therefore isolates its descendants even when each child omits an
instance key. A `var` is private to the Operator instance that declares it;
other Operators communicate through ports rather than reading that cell.
See [Instances, state and time](../guide/06-instances-state-and-time.md).

Mailboxes and Call capacities have bounds. A source Connector's proposal
remains pending when its target mailbox is full; direct App input admission
instead reports a capacity error. Neither commits an ingress before room is
available. Adding an instance does not remove capacity bounds. The number
of pending Calls is not a count of parallel Operator transactions.

## What the two-tenant example demonstrates

In [Two tenants](../examples/03-two-tenants.md), Alice's first Call remains
pending while Bob's Call completes. Alice's timeout and backoff belong to
her instance, so Bob does not repeat his request. The Lab supplies both
partners' behavior and advances the clock explicitly. This demonstrates
independent progress and isolated Retry state, not multicore throughput.

The corresponding real-world pattern can have several external requests
outstanding. A Process Connector may start child processes under its
Contract; that does not make every keyed Operator an operating-system
process.

## Ordering and replay

Deliveries within one instance mailbox are consumed in FIFO order.
Independent instances have no application-level ordering guarantee between
them. When external responses race, their admitted order can differ between
live runs. The journal records the chosen order and causes.

The Lab makes inputs, partner responses and time explicit, so a scenario
can reproduce its journal. Diagnostic replay reads recorded facts without
repeating external effects. Neither claim means that live partners must
respond in the same order on every deployment. See
[The Lab](lab.md) and [Running for real](../guide/05-running-for-real.md).

## Specified, not shipped

| Capability | Boundary of the current implementation | Specification |
|---|---|---|
| A bounded pool of Workers across CPU cores | The hosted runtime uses one Worker; keyed instances are not scheduled across a pool | `docs/spec/execution-model.md` §2 |
| Distributed placement and reassignment | Virtual-shard mapping has a proof implementation, but the alpha does not deploy instances across Hosts or implement ownership leases | `docs/spec/instances.md` §9 |
| A separate compute pool | Operator evaluation is synchronous; CPU-heavy work is not offloaded by the runtime | `docs/spec/execution-model.md` §2 |

These implementation limits do not change the ownership rule: one instance
must never execute conflicting transactions concurrently.
