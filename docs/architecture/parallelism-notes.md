# Parallelism — logical instances and physical execution

> Design notes — non-normative, extended 2026-09-17. Logical Executor/manager
> ownership is implemented in v17; multi-Worker execution remains future work.
> The semantic authorities are
> [instances.md](../spec/instances.md),
> [extensions.md](../spec/extensions.md), and [loop.md](../spec/loop.md).
> R9 removed Graph application handlers; Loop commands never enter the spatial DAG.

## Four concepts that must stay separate

```text
instance.key       = logical address and serialization domain
lifetime            = when one logical instance closes
Connector capacity  = how many external calls may be open
workers / shards    = physical execution and placement
```

None is shorthand for another. A key never means “make a thread”; a lifetime
never defines durability; Connector concurrency never weakens the one-turn
mailbox rule; placement never changes program meaning.

## The invariant shared by embedded and clustered runtimes

One logical **Executor** instance owns one bounded delivery mailbox. At most
one worker claims that mailbox for a transaction at a time. Its immutable
bindings, semantic state, open exchanges, lifecycle, armed events and consumer
cursors share the same canonical instance reference `(logical address,
generation)`. An Operator's semantic state is its DSL `var`; a Connector's is
the core-owned Contract/exchange fold. Physical Driver handles are rebuilt and
are never semantic state.

A Graph instance retains address, lifecycle and containment but owns no
application mailbox or transaction. Boundary admission selects and reserves
its contained routes. A temporal Extension manager is also not a Node: it has
a bounded durable-command inbox per run and serializes its transactions at the
owner's address. Graph ownership and manager control therefore create no
second spatial scheduler domain.

```text
today:  one deterministic pump
later:  bounded worker pools on several Hosts
always: one Executor reference -> one claimed mailbox -> one transaction at a time
        one manager command -> one owner/run transition at a time
```

An unkeyed owner is a singleton inside each enclosing Graph instance. A keyed
owner has one logical address per canonical selected key and a fresh generation
after each completed close. Order is FIFO within one instance reference and
deliberately unspecified across independent references. This is enough to
keep Executor state lock-free in the reference machine and single-writer in
the distributed machine.

Several Connector calls may nevertheless be open for one Node instance.
Opening a call is one short transaction that journals its obligation; the
mailbox may then process another delivery if the resolved Connector contract
allows it. Call identity correlates every `out`/`err` frame with exactly one
typed `exit`. Instance Closing separately stops admission and boundedly
drains/cancels/settles those calls.

## Tasks are a runtime choice, not an instance's meaning

The logical scheduling unit is an Executor mailbox or manager command inbox;
the atomic unit is one transaction. An async task per live instance is one
possible implementation, not a promise to allocate a task, thread or process
for every key. Scheduling ready mailboxes on a bounded pool is another. A
Graph remains a scope under either strategy; its children do not acquire a
shared transaction or a graph-wide execution lock.

The following are future implementation profiles, not new language modes:

| Profile | Physical execution | Status |
|---|---|---|
| Embedded/reference | Bounded ready queues, one cooperative Worker, explicit Driver polling | The current one-Worker model; no autonomous task per instance |
| Hosted multicore | Bounded Workers, optionally async tasks for suspended physical work | To design and qualify |
| Multiprocess / distributed | Placement of instances or groups, explicit cross-Worker transport and ownership fencing | To design and qualify |

`no_std` describes the library boundary, not the scheduler: an embedded host
may have an RTOS or multiple cores. Conversely, a hosted runtime can use the
same cooperative reference engine. An open Call or resident Process is an
external obligation, not a DSL transaction waiting for the entire operation
to finish. Other ready work may run while the Driver waits. Heavy computation
inside one transaction still needs an explicit execution budget; cooperative
execution alone is not a latency guarantee.

All profiles must preserve per-instance ordering, atomic reservations and
commits, exact-generation Closing and journal-relative replay. Different live
schedules need not produce identical interleavings: competing arrivals are
retained, not reconstructed from the replacement scheduler.

## Independent flows are placement candidates, not another Executor

The local chat example has two sibling flows inside one App: an HttpServer
returns an embedded HTML asset, while a WebSocketServer routes conversations
to resident Processes. Both listeners are shared; only the conversation scope
is keyed. Loading the page does not wait for Alice's model, and Bob can use a
different child while Alice's child is working. The existing one-Worker VM and
non-blocking Driver boundaries are sufficient for this demonstration. This is
I/O concurrency, not evidence of parallel Operator execution.

These flows may later be grouped or isolated for placement and resource
budgets. They do not require a second mandatory task layer above instances.
One task for the whole chat flow could unnecessarily serialize all users;
one task per drawn Graph would confuse lexical containment with scheduling.
Use *independent flow* rather than *closed circuit*: the spatial graph is
still a DAG, and physical request/reply traffic does not add spatial cycles.

Disconnected spatial components are only candidates for such grouping. A
shared lifecycle parent, manager commands, an exchange's request/response
correlation, shared budgets and the journal can still require coordination.
A partitioner must account for those dependencies, not merely erase edge
directions and find connected components. No grouping syntax, automatic
partitioner, scheduling-island identity or new plan field is decided here.

## Hierarchical address, independent placement

Nested owners produce hierarchical logical addresses, but every keyed owner
is a routing boundary that may shard independently of its parent. Otherwise
one hot outer key could trap all inner keys on one shard.

```text
logical address
    -> stable virtual shard
    -> current Worker owner
```

The mapping from logical address (generation excluded) to virtual shard is
stable and architecture-neutral. R3 pins the proof seam as the first eight
bytes of BLAKE3 over the canonical address, interpreted unsigned big-endian
and reduced modulo a fixed non-zero virtual-shard count.
Worker count may change without changing that mapping; changing the virtual
shard count is an explicit reshard migration. A cross-keyed-boundary delivery
is journaled even when both endpoints are currently colocated. Worker, Host,
lease, and leader epoch are transport facts and never causal identity.

A parent close is therefore specified as durable lifecycle messages to its
known children, completed inner-first. A one-process runtime may optimize that
protocol into local calls without deleting its journal facts.

## Decisions still required before production parallelism

1. **Shard ownership and fencing** — leases, migration, and reconstruction
   without two live writers for one address.
2. **Journal serialization** — global commit sequencer or per-shard journals
   with causal order across shards. Replay remains journal-relative.
3. **Live-cardinality admission** — runtime caps, waiting/backpressure, and
   deterministic eviction policy. This is not source `instance.key`.
4. **Connector bulkheads** — max open calls and queue policy, separate from
   the number of live logical instances.
5. **Explicit re-key boundaries** — changing correlation/addressing is a
   visible keyed Node and journaled delivery, never a silent compiler insert.
6. **Long CPU work** — small intrinsic expressions run in the owner turn;
   heavy guests use a bounded compute pool while logical ownership is held.
7. **Hot-key operations** — mailbox depth and lag first; no speculative
   salting that silently breaks order.
8. **Task granularity and fairness** — ready-mailbox scheduling versus async
   tasks, bounded activation batches, and protection of cold keys and HTTP
   ingress from a hot conversation. Measure latency and memory before choosing.
9. **Flow grouping** — optional colocation/isolation policy, dependency-aware
   cuts, shared listener ownership and budgets; never a new atomic Graph unit.

These are post-M1 design gates. Completing the local HTTP/WebSocket chat does
not depend on them and must not be reported as implementing them.

## Kafka Streams translation

```text
Rastok selected key          ~= stream key / locality domain
Rastok logical Node instance ~= keyed task or virtual actor
Rastok virtual shard         ~= physical assignment unit
Rastok journal               >  a state changelog: it also seals effects,
                                 timers, call exits, and lifecycle
```

A join correlates within the owning Operator instance. If two streams need a
different correlation domain, an explicitly keyed owner performs the re-key;
the new edge/address is causal and can be placed independently. A skewed key
still creates the familiar hot-key bottleneck. Adding Workers helps different
addresses, never parallelizes two transactions in the same mailbox.

## The one invariant that must never drift

No implicit concurrent transaction inside one mailbox. It would break FIFO,
Executor-owned state, derived identity, external-effect ordering, and replay. Any
future simultaneous protocol activity is an explicit Connector contract over
individually correlated calls, not a scheduler shortcut.
