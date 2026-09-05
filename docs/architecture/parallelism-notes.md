# Parallelism — logical instances and physical execution

> Design notes — non-normative, forward-looking. Updated after
> D-2026-09-05-1 for the v16 target. The semantic authorities are
> [instances.md](../spec/instances.md),
> [extensions.md](../spec/extensions.md), and [loop.md](../spec/loop.md).
> Current v15 Graph Loop code retains its historical execution path until R7.

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
