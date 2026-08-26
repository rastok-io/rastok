# Instances, keys, lifetime, and placement

> **Status:** normative M1.4 target, adopted by D-2026-08-18-1. The compiler
> accepts and checks the R1 `instances` / `lifetime` surface. Plan-package v12
> retains v9's categorical parent-linked owner table, scoped routes,
> independent instance policy, immutable owner expressions, and phased
> Connector settings. R3 routes every owner through canonical hierarchical
> instance references; those references index routes, transactions,
> mailboxes, state, calls, timers, lifecycle, and cursors. R4 executes all five
> closed lifetime profiles on Graph and Node owners, including global
> temporal singletons. R5 executes bounded Closing, successor-generation
> admission, cancellation, settlement, terminal `OutcomeUnknown`, inner-first
> Graph closure, and bare `stop`. Source production of some dynamic
> settings/values and Graph handlers remain named R1 gaps. R6 makes nested
> Graphs and instance-local joins public end to end, including Lab and package
> evidence. R7 closes the executable slice with negative-space and
> placement-neutrality proofs; it adds no cluster runtime. This document is
> the single authority for logical instance addressing, key selection,
> Node-local state, lifetime, and the semantic boundary with future sharding.

Rastok separates three concerns that the former `partition` declaration
combined:

1. **addressing** — an optional key selects a logical Node instance;
2. **time** — an optional lifetime determines when that instance closes;
3. **scope** — lexical containment determines which immutable names are in
   scope, while mutable state remains owned by one Node.

Placement is a later physical decision. It never changes those semantics.

## 1. One owner, three independent attributes

Every Node family — Operator, Connector, or Graph — may declare an instance
key on its definition:

```rastok
operator counter {
  instances = {
    key = meta.app.customer_id
  }

  var count: long = 0

  on in {
    count = count + 1
    out = count
  }
}
```

`instances`, `lifetime`, and `closing` are separate common
Node attributes:

```rastok
graph conversations {
  instances = {
    key = meta.app.session_id
  }

  lifetime = {
    kind = idle
    after = duration("2h")
  }

  closing = {
    drain_for  = duration("30s")
    settle_for = duration("5s")
  }

  ...
}
```

Their absence has one meaning each:

- no `instances` means one singleton instance in the enclosing scope;
- no `lifetime` means that instance is persistent;
- no `closing` means no drain/cancel/settlement budget is invented. Every
  finite owner whose subtree contains a `Process` must declare it; an owner
  without such calls may omit it and closes at natural quiescence. A persistent
  owner may declare it for explicit `stop`. Both durations are positive and
  required; there are no action fields or defaults.

There is no `partition` declaration, named or inline, and no `per` binding.
The owner declares its key once. A declaration site never reselects another
Node's state by name.

## 2. Key selection

`instances.key` is evaluated exactly once for every envelope crossing into
that owner. It runs against the complete immutable entering envelope:

- `data`;
- `meta.sys`;
- `meta.app`;
- `meta.ext`;
- immutable `val` bindings from enclosing lexical scopes.

The owner's own body is entered only after its address has been selected, so a
key may not depend on a `val` declared by that same owner. Descendant Nodes may
consume the owner's `val`s normally after entry.

It cannot read destination facts that do not exist yet: the selected shard,
arrival sequence, commit time, target mailbox, target state, or target
`instance.key`.

The M1.3 port solver first resolves every effective input type. The checker
then type-checks the key expression under every input port that can admit an
envelope. Every projection must be total and produce the same key type;
otherwise the Node refuses at check. Source omission of a port annotation
never turns the key into `variant` and never creates a second inference
engine. For an inbound source that creates an instance directly from an
accepted world envelope, the Connector contract's resolved ingress-produced
envelope type is the admission signature and is checked by the same rule; the
absence of a graph input port is not an untyped escape.

The type stored beside the key is never trusted or silently repaired. Source
lowering has one explicit privilege: after total port inference it materializes
the type of a key that reads `data`. A free raw plan and a decoded package must
already carry the exact type re-derived from the expression, every admitting
input, the transported `app`/`ext` schemas, the fixed `sys` spine, and visible
ancestor `val`s. A different claim is a validation error even when both types
belong to the legal key domain.

The selected value is canonicalized before target mailbox capacity is
reserved and before the producing transaction commits. Reservation returns
the destination's current generation. The producing record stores the
resulting **instance reference** — logical address plus generation. Replay
consumes that recorded decision; it may re-evaluate the expression as an
integrity check, never as routing authority. Changing `data` or metadata after
entry never migrates the current instance.

`meta.sys.id` is therefore a valid key and naturally means one instance per
entering envelope. Inbound-envelope identity is minted by its logical producer
before target routing; the provenance replica is not the target shard.

The **selected** key is exposed as contextual `instance.key`, not copied into
`meta.sys`. `meta.sys` belongs to the immutable envelope and travels unchanged;
one envelope may visit several owners with different addresses. Writing a
hop-local address into it would either mutate the envelope or leave a stale
key on the next hop. The journal records each selected address instead.

Entering nested owners is an outermost-to-innermost pipeline, never one
recursive expression evaluation over half-created state. For each newly
entered owner, the runtime selects its key from the immutable entering
envelope and already-established enclosing `val`s, reserves that owner's
instance reference, then purely validates its per-instance bindings. Only
then may the next child key observe those enclosing bindings. No key or
binding may observe a child that has not yet been selected. A committed route
therefore names one final target instance reference while the validated owner
tree and journaled parent/child lifecycle facts retain the chain that led to
it.

An inbound source follows the same rule. Its boot adapter may accept and mint
an immutable envelope, which then selects the entered owner chain. Any
Connector field required to open the listener, poller, or other source before
that envelope exists must close no later than boot. An instance-site field is
legal only after an entering envelope has selected that Connector instance;
it cannot configure the bootstrap resource that produced the envelope.

### 2.1 Canonical key domain

A key is either:

- one closed value in the canonical map-key domain of [types.md](types.md);
  or
- a closed, recursively nested `struct` whose leaves are in that domain.

Canonical scalar/enum leaves include booleans, fixed-width integers, text,
bytes, decimal, temporal values, UUID, and enum. Float, `number`, null,
optional fields, interval, `variant`, union, array, map, table, and opaque
values refuse. A missing key or missing struct field is an error, never a
distinguished `missing` key.

Struct field order in source is not identity. The selected-key frame seals the
complete resolved key `DataType` beside its value; canonical struct encoding
sorts fields by name at every level. Thus field names, resolved leaf types and
canonical values are all identity, and these select the same instance:

```rastok
instances = { key = { tenant = data.tenant, session = data.session } }
instances = { key = { session = data.session, tenant = data.tenant } }
```

but a renamed field selects a different address. The inferred resolved key
type travels in HIR and the validated plan even though source carries no
separate type annotation.

## 3. Logical address, instance, and generation

An instance's logical address is the canonical outermost-to-innermost stack:

```text
static owner path
+ every dynamic ancestor frame
+ the owner's local selected-key and/or time-box frame, when present
```

Example:

```text
tenant="acme" / session="s-42" / summarize
```

Singleton ownership contributes no dynamic frame; the static owner path still
distinguishes Nodes. The exact canonical CBOR sub-form and extend-only frame
kinds live in [identity.md](identity.md).

Compilation may flatten composition to validate the spatial DAG, but the
validated plan preserves every Graph, Operator, and Connector as an owner and
preserves the parent relation between them. A route retains its lexical Graph
scope. The runtime can therefore derive which owner frames a delivery leaves
and enters from the route plus the owner tree; it never reverse-engineers
containment from dotted path text.

An address names where deliveries route. An **instance** is the currently
live state and lifecycle at that address. After it closes, a later delivery
may create a new instance at the same address with clean Node-local state.

The runtime distinguishes successive instances by a **generation**. It is the
zero-based ordinal of `l:start` for that address, derived from the journal and
never from a RAM counter. An **instance reference** is the pair `(logical
address, generation)` used by routes, transactions, mailboxes, state, and
lifecycle facts. Generation fences stale deliveries, timers, lifecycle
messages, and old ownership. It is not part of the logical address, call
identity, or source vocabulary.

A processing-time-box address has at most one forward-execution generation:
the next tumbling box has a different address frame. Idle, fixed, single-pass,
and explicit stop may instead close an address that a later admission reuses
with the next generation.

`instance.key` exposes the owner's local selected key as a read-only value in
that owner's handlers. It is statically available only when that owner declares
`instances.key`; it never falls back to an ancestor key or to null. A Graph may
bind immutable values derived from it:

```rastok
graph workspace {
  instances = { key = meta.app.workspace_id }
  val cwd = "/workspace/" + instance.key
  ...
}
```

Those `val`s are lexically visible to descendants. They are immutable context,
not remotely read state.

The initialization phases are distinct:

- a unit/App boot `val` is resolved once for the deployment and may use the
  boot-only `env()` / `secret()` functions;
- a `val` declared inside any other Node is derived once for each new Node
  instance from enclosing immutable bindings and, when declared, that owner's
  `instance.key`; it cannot read `data`, `meta`, or mutable state;
- a handler-local `val` is evaluated for that transaction and may read its
  ordinary handler bindings.

Per-instance `val`s are deterministic derived context, not journaled mutable
cells. Their typed expression is semantic plan identity; their evaluated value
lives in the instance bundle.

Connector contracts name the **latest evaluation site** allowed for each
configuration field. The ordered sites are plan, boot, instance, and
invocation. The checker always chooses the earliest site at which the written
expression closes and transports that actual site in the effective plan. An
attribute that selects a profile, port type, effect, behavior, or other plan
shape has `plan` as its latest site and must therefore reduce during
compilation. A contract may allow an operational setting such as a command,
argument, URL, environment value, or a path shared by an instance as late as
**instance**; its
typed expression may then read enclosing immutable `val`s and the Connector's
own `instance.key`, and is evaluated exactly once when that Connector
instance binds. A literal still reduces at plan, and an `env()`-only
expression binds at boot. The validated plan carries whether an effective
attribute is a resolved value or a boot/instance/invocation expression, so an
adapter has no Registry or fallback decision to make.

An attribute whose latest site is invocation, such as `File.path`, an
idempotency key, or a correlator, may read that invocation's typed envelope.
It is not instance
configuration and cannot participate in instance addressing. Boot, instance,
and invocation expressions all use the same expression tree; their signatures
differ and the checker rejects a name from the wrong phase. An expression may
not combine `env()` directly with instance or invocation bindings: bind the
boot value to an enclosing `val`, then consume that immutable value later.

```rastok
val WORK_ROOT = env("WORK_ROOT")

graph sessions {
  instances = { key = meta.app.session_id }
  val workdir = WORK_ROOT + "/" + instance.key

  connector worker : Process {
    command = "worker"
    cwd = workdir                 // Process marks cwd instance-bound
    codec = jsonl                 // plan-resolved: it determines port types
    effect = idempotent
    behavior = deterministic
  }
}
```

The contract sheet, not expression shape, owns that distinction. A dynamic
value is never accepted for a plan-shaping attribute merely because it happens
to reduce for one instance.

## 4. Mutable state has one Node owner

A `var` belongs to exactly the Node that declares it and is visible only in
that Node's handlers. It is indexed by that Node's complete instance
reference.
Transactions of one instance read the previous committed state, update it
imperatively, and commit state plus emissions atomically.

A Graph does not expose shared mutable state to child Nodes. A child may use
the same field spelling; it denotes an independent cell owned by that child.
There is no graph-wide transaction, snapshot, lock, or hidden shared memory.

Shared mutable behavior is an explicit stateful Operator with typed command
and result ports. All operations that must be atomic over its state belong in
one of its handlers:

```rastok
operator account {
  instances = { key = meta.app.account_id }
  var balance: long = 0

  on debit {
    balance = balance - data.amount
    out = { balance = balance }
  }

  on credit {
    balance = balance + data.amount
    out = { balance = balance }
  }
}
```

This preserves the node transaction as Rastok's only ACID unit. Keying a Graph
does not make an envelope's whole traversal a transaction.

## 5. Mailboxes and ordering

Every logical Node instance owns one bounded mailbox. Its mailbox, derived
immutable bindings, `var` state, open calls, lifecycle, armed events, and
consumed cursors use the same instance reference.

One mailbox is claimed by at most one worker at a time. Transactions are FIFO
within an instance. Order across independent addresses is deliberately
undefined and cannot be observed through shared mutable state, because none
exists between Nodes.

An unkeyed Node is a singleton inside each enclosing Graph instance. It does
not re-evaluate an ancestor's key; the ancestor frames already occur in its
address.

## 6. Lifetime

Key and lifetime are orthogonal:

| `instances.key` | `lifetime` | Meaning |
|---|---|---|
| absent | absent | persistent singleton |
| present | absent | persistent instances by key |
| absent | present | temporal singleton/window within the enclosing Graph address |
| present | present | keyed sessions/windows |

The lifetime profiles use the common composite-attribute form:

```rastok
lifetime = { kind = persistent }
lifetime = { kind = idle       ; after = duration("2h") }
lifetime = { kind = fixed      ; after = duration("30s") }
lifetime = { kind = single_pass }
lifetime = { kind = tumbling   ; every = duration("5m") }
```

Omission is canonically equivalent to `kind = persistent`; lowering removes
the spelling difference before semantic identity.

The table is the closed executable R4 vocabulary, not merely a transport
registry. Graph and Node owners execute all five profiles. Sliding and
event-time windows remain named later gates rather than inheriting one of
these meanings.

### 6.1 Persistent

No deadline is armed. Explicit stop, deployment shutdown, or supervision may
still request closure under the bounded closing contract.

### 6.2 Idle

Mailbox admission is activity. Admission records the causing envelope and
arms `last_admission + after` with the current journal-derived instance
reference. The unique causal last-admission id additionally fences a deadline
superseded within that generation. A later admission supersedes the earlier
cause; a later generation cannot be mistaken for the expired one.

Idle measures admission silence. An armed call, Loop backoff, or other dated
obligation does not silently rewrite that definition. If the idle deadline
wins, the instance enters Closing; its outstanding obligations are then
drained, cancelled, settled, or retained according to their own protocol and
the closing contract. Expiry itself never suppresses a Connector's terminal
`exit` or a Saga compensation.

Every `Process` in the subtree of a finite owner requires an explicit
`closing` policy on that owner. Its per-call `timeout` remains independent and
optional: it bounds an individual invocation during normal operation, while
`closing` bounds how the whole instance retires all admitted work.

### 6.3 Fixed

The creating admission arms `start + after`. Later traffic does not move the
deadline. At expiry the instance enters Closing. The dated event carries the
complete generation-bearing instance reference, so an old event cannot close
a later generation at the same address.

### 6.4 Single pass

One root admission creates the instance and durably requests Closing. Further
root admissions reserve the exact successor generation and wait in its normal
bounded mailbox. The current generation drains its admitted work under the
same explicit closing contract as every other finite lifetime.

### 6.5 Tumbling

Processing-time boxes are epoch-aligned at `epoch + k·every`. The runtime adds
the canonical window start as a runtime-owned address frame; it is not a field
the source author fabricates. One envelope belongs to exactly the box open at
its admission. A lifetime without `instances.key` is therefore a true temporal
singleton in each enclosing Graph address, rather than a fake constant-key
partition; it is App-global only when every ancestor is singleton.

Each box address admits generation zero at most once in forward execution. A
route committed before box closure remains old-box work and drains there; an
attempt after closure cannot reopen that address as generation one or spill
into the next box. The next epoch boundary creates a different address frame.

Sliding windows remain a named deferred feature: overlap maps one envelope to
several instances and therefore requires explicit one-to-many admission.
Event-time and watermarks are a separate additive layer.

## 7. Start, stop, and containment

The first admission to an absent address reserves the next journal-derived
generation. Every selected Graph and Node owner records exactly one lazy
`l:start`, including persistent singletons; persistent only means that no
deadline is armed. Before `l:start`, the runtime purely evaluates and validates
that owner's per-instance `val`s and instance-bound Connector settings into a
candidate bundle. A failure leaves the delivery unconsumed, commits no
`l:start`, and performs no Driver effect. Once the bundle is valid, the
runtime commits the `l:start` lifecycle transaction before the port
transaction or any external resource effect. `on start` and `on stop` are
ordinary handlers owned by the Node and require a causal input envelope;
Graph lifecycle facts execute now, while Graph handler source/lowering remains
the named R1 gap.

Bare `stop` requests closure of the current Node instance after the current
transaction commits. There is no `stop <partition>` form. Stopping a Graph
also requests closure of every live descendant instance created in that Graph
generation; children finish before the parent's `on stop`.

Parent/child lifecycle is not shared memory. Each child `l:start` opens a
runtime lifecycle obligation toward its parent generation by carrying that
parent’s complete instance reference; `l:stop` closes it. The logical child
address cannot substitute for this fact because it deliberately excludes
ancestor generations. This relation is a fold of journal facts. It lets a
future distributed parent enumerate children and send durable close requests.
The journal refuses a child start under an inactive parent and refuses a
parent stop while any direct child remains active; because every deeper child
has an active direct ancestor, this enforces the whole inner-first tree.

A child creation racing with parent close is decided by the journal:

- admitted before the durable `l:closing` fact: it exists and joins the close
  cascade;
- observed after: it cannot start under the closing parent generation and is
  routed, subject to ordinary bounded-mailbox capacity, to the exact successor
  generation.

The same close-once arbitration governs every route. **Admission** means the
producer has reserved destination capacity and atomically committed the route
with the returned instance reference; it is not the later instant at which a
worker happens to poll the mailbox. A route admitted before `l:closing`
names that generation, counts as work in its drain, and can never be delivered
to its successor. A route attempted after the close won reserves the exact
successor generation and commits into that generation's ordinary bounded
mailbox; it becomes runnable only after the predecessor's `l:stop` commits and
state is reclaimed. No second waiting queue exists. This distinction also
prevents a delayed inter-shard transport from reopening an expired tumbling
box. A `single_pass` generation can reserve only its immediate successor; a
further root admission receives ordinary backpressure until that successor
starts, because generation numbers are journal facts rather than speculative
RAM counters.

### 7.1 The bounded Closing contract

The complete sheet is:

```rastok
closing = {
  drain_for  = duration("30s")
  settle_for = duration("5s")
}
```

Both fields are required positive constant durations. `drain_for` is measured
from the committed close request. `settle_for` starts when that drain interval
ends. The runtime actions are fixed by this contract; authors do not select
`kill`, `wait`, or `otherwise` branches, and omission never creates hidden
budgets.

The first causal close request wins once in the journal. The committed
`l:closing` transaction names the root instance reference of the closing
scope, seals the two dated obligations, and closes external admission to that
generation. A Graph scope includes every currently live descendant. Its one
policy bounds the whole subtree; child policies govern independent child
closure, not a parent cascade, so nested budgets neither add nor silently
extend the parent's bound.

Closing then follows this sequence:

1. During `drain_for`, already admitted deliveries and completions inside the
   scope continue. New external deliveries go to the successor generation.
2. At the drain deadline, unconsumed internal deliveries become residuals of
   the closing generation. Their consumed cursor positions are sealed by its
   final stop transaction, so they are visible and cannot leak into the
   successor. Open calls are cancelled in canonical `CallRef` order.
3. During `settle_for`, an observed completion still wins the ordinary
   close-once race and publishes its normal typed `exit`.
4. At the settlement deadline, each call still open is atomically marked
   `abandoned` and publishes the Connector contract's empty
   `OutcomeUnknown` case on its ordinary `exit` port with trigger
   `l:recover`. A later partner response is a discarded observation, never a
   second causal result.
5. Descendants commit `l:stop` inner-first. Each `on stop` runs only after the
   last required call exit and residual decision in its scope. State and the
   instance bundle are reclaimed only after that stop commits; the waiting
   successor may then start with clean state.

An observed cancellation result is not rewritten as `OutcomeUnknown`: for
`Process`, a confirmed kill normally yields `Exited { ... }`. The unknown case
means only that no terminal outcome was confirmed before settlement expired.
It has no `reason` payload; its causal trigger and the abandoned OUTBOUND fact
carry the provenance.

Loop wakeups and backoffs are generation-bound and die with their instance.
Saga compensation debt is journal-bound and survives instance closure; saga
state remains a fold of ordinary journal facts, never a second ledger.

## 8. Joins

A join buffers and correlates inside the logical instance of the Operator
that owns its handler. Presence joins zip arrivals in that instance's mailbox
order. The delivery that first completes presence is the causal trigger; the
tuple itself stays in source-declared port order. With `where`, FIFO candidate
tuples are tested lexicographically and a false predicate consumes nothing.
`within` and `last` bound retained candidates independently: the former arms a
dated obligation per delivery, while the latter evicts the oldest excess
candidate per port. In the M1.4 surface either bound requires the conventional
typed output `expired: union { JoinExpired {} }`; eviction emits that ordinary
value with `l:evict` before consuming the retained cursor, never a silent drop.

There is no implicit current envelope under a join. Bare `data` and
`out = .` refuse; `a.data` and `a.meta` select the complete input named `a`.
Each named payload has its own lazy materialization slot, so inspecting one
encoded input never decodes another. Writable metadata uses CommonOrAbsent
over present values; an explicit write from a named input wins. The plan
validator conservatively propagates possible absence through topology and
rejects a later required bare read until such a choice is made. Optional
access remains a separately named expression-language gate.

There is no `by` override. A different correlation domain is a different
owner key and should be expressed by giving that Operator the appropriate
`instances.key`. This keeps routing and correlation under one instance model.

A partial join is retained state but not runnable work. It is therefore
quiescent for `drain`/Lab stability until another arrival, a `last` overflow,
or its dated `within` event makes a transaction runnable. Retention never
creates another address, generation, or shard key.

## 9. Placement and shards

Every keyed owner is a logical routing boundary. Its address is hierarchical,
but by default it may be assigned to a shard independently of its parent.
That prevents an outer hot key from forcing every nested key onto one shard.

Crossing that boundary is always a delivery: the producer records the target
instance reference, then the destination mailbox consumes it. The address
part alone selects the virtual shard; the generation fences ownership within
that shard. A runtime may turn a colocated delivery into an in-process call,
but it may not remove the causal facts. Future colocation policy is a physical
optimization, not source semantics or semantic identity.

The three layers are distinct:

```text
logical instance address
        -> stable virtual shard
        -> current Worker owner
```

The current proof seam hashes the exact canonical address bytes with BLAKE3,
interprets the first eight digest bytes as an unsigned big-endian integer,
and reduces it modulo the fixed non-zero virtual-shard count. This mapping is
architecture-neutral: it never hashes a Rust enum or host-sized integer.

The virtual shard count is stable and independent of Worker count. Adding or
removing Workers changes only shard assignment. Changing the virtual shard
count is an explicit reshard migration. Worker, Host, lease, and leader epoch
never enter causal records; logical ingress producer identity is not target
placement.

Parent/child close across shards uses the durable lifecycle obligations from
§7. Local execution is the degenerate form of the same protocol.

## 10. Supersessions and compatibility

D-2026-08-18-1 supersedes the unreleased named-partition model, including:

- `partition name { key = …; lifetime = … }`;
- inline partitions;
- `per name`, `per a, b`, and `<partition>.key`;
- standard predeclared partitions;
- nominal cross-partition state reads;
- shared Graph-level mutable `var` state;
- `stop <partition>`;
- join `by expr`;
- the claim that every key necessarily shares one lifetime declaration;
- the claim that a nested keyed scope must share its parent's placement.

It preserves the useful invariants already proved by the P0–P3 draft:

- one canonical key byte authority;
- route selection before producer commit;
- journaled target authority;
- FIFO mailbox and state isolation per instance reference;
- causal `l:start` / `l:stop`;
- generation-fenced dated events;
- one synchronous Worker as a valid embedded/reference implementation.

The plan/package and journal vocabulary has been rewritten from the draft
`partition_key` / `target_partition` names to logical instance addresses and
generation-bearing references without renumbering already reserved
extend-only journal keys merely for spelling.
Plan-package v12 retains v9's categorical owner/instance transport and
refuses v11 and every older format, including the unpublished v8 draft, as
input. The original v9
package-only move left semantic hashes and causal traces unchanged at R2. R3
made the instance reference
mandatory in the causal journal core as its own declared trace move: existing
trace goldens move, including singleton graphs, while semantic graph hashes
and plan-package bodies remain unchanged.
