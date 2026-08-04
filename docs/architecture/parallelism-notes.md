# Parallelism — what M0 bought, and what remains to decide

> Design notes — non-normative, forward-looking. Recorded 2026-08-04
> at M0 exit, after an adversarial design review of the concurrency
> model. Nothing here changes M0 semantics; everything here names a
> decision the next slices must close spec-first.

## The mental model, in four separated concepts

```text
per              = business independence     (a property of the domain)
max_instances    = live cardinality          (how many keys exist at once)
max_concurrency  = execution capacity        (a Connector's bulkhead)
workers / shards = physical placement        (invisible to the program)
```

These four are deliberately not one knob. `per` never means "threads";
capacity never leaks into the language; placement never changes what a
program observes.

## What M0 already enforces (code, not intention)

Logical parallelism is `(node, partition_key)`; physical parallelism
is whoever executes those instances. The invariants below are built
and tested in the M0 machine:

1. FIFO order is defined per `(node, key)` — one mailbox, all ports,
   arrival order.
2. No order is promised between independent keys.
3. State is isolated per key (`PartitionState`).
4. An I/O wait frees the worker but keeps the mailbox **claimed** —
   the claim is an explicit concept (`Mailboxes::claim/release`), not
   an accident of single-threading.
5. Mailboxes are bounded; backpressure refuses before any fact is
   committed, and never loses a delivery.
6. Observable races are settled and journaled (close-once; every
   non-causal observation is returned, never dropped).
7. `per` expresses a business property.
8. `max_instances` bounds live cardinality, not intra-instance
   concurrency.

Because "one active transaction per mailbox" is guaranteed twice today
— by the single-threaded scheduler AND by the claims — replacing the
M0 scheduler with a worker pool removes the first guarantee while the
second becomes the lock. The migration path to shards and pools
changes no user-facing semantics:

```text
today:  BTreeMap of mailboxes + one logical worker
later:  shards + worker pool + processes/machines
always: one mailbox = (node, key) ; one active transaction per mailbox
```

M0 deliberately stays below the model's capacity: the key is the
singleton `""`, one deterministic worker, no executable `per`, no
effective `max_instances`, no joins, no placement.

## Decisions to close before production parallelism (spec-first)

1. **Physical key placement** — shard ownership, migration and
   rebalancing, fencing against two simultaneous owners,
   reconstruction from the journal. Changes nothing in the
   programming model.
2. **The journal as serialization point** — today the total order of
   commits is a by-product of the single writer. With workers, the
   order becomes a decision: a global sequencer, or per-shard
   journals with only causal order across shards. Replay determinism
   is journal-relative, so this decision defines what "replay" means
   when distributed.
3. **Saturation at `max_instances`** — what happens to key N+1: wait,
   backpressure, typed refusal, or idle-instance eviction. Must be
   deterministic and observable.
4. **Connector capacity (bulkhead)** — `max_instances` is not an
   admission policy: a thousand live sessions must not imply a
   thousand simultaneous calls. A distinct, declared capacity
   (max_concurrency, queue depth, admission policy) — never hidden
   inside an adapter.
5. **Co-partitioning and re-key** — future joins require both inputs
   keyed compatibly; the checker guarantees it or demands an explicit
   redistribution. This chantier owns the DeliveryRef → CallRef
   formalization already recorded in
   [`m0-exit-notes.md`](m0-exit-notes.md): identities must see the
   delivering edge before two ports or two instances can receive one
   envelope.
6. **Long CPU work** — small CEL runs on the worker; heavy guest
   programs (jq/SQL) go to a bounded compute pool. The partition
   stays reserved; the general scheduler is never monopolized.
7. **Hot keys** — telemetry first, guidance after; no speculative
   mechanism.

## The one invariant that must never drift

No implicit concurrency inside one mailbox, ever. Two envelopes of
one key executing in parallel would break the order, the state, the
derived identities, the external effects, and the replay — the entire
causal apparatus assumes the sequence. If an instance must someday
accept simultaneous calls, that is an explicit Connector contract
with its own correlation and ordering rule, not a scheduler
optimization.
