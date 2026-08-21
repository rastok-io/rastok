# Design decisions

> One entry per settled decision: what was chosen, why, and what was
> rejected — so a rejected alternative is not re-litigated without new
> evidence. Companion to [`errata.md`](errata.md) (defects found);
> git history carries the full context of each entry.

## D-2026-08-18-1 — Nodes own instances; key, lifetime, and state scope are separate

The unreleased named-partition model fused three independent concerns: a key
selected an execution address, a lifetime ended it, and a partition name let
distant Nodes reach state outside topology. Rastok now assigns each concern
to its natural owner. The complete contract is
[instances.md](instances.md); this entry records the decisions and named
supersessions.

**A Node declares its own addressing.** Any Operator, Connector, or Graph may
carry the common composite attribute `instances = { key = <expr> }`. Its key
expression is evaluated exactly once for each envelope crossing that Node's
boundary, against the complete immutable envelope (`data`, `meta.sys`,
`meta.app`, `meta.ext`) and lexical immutable values. Destination facts are
unavailable. The canonical selected instance reference — address plus the
generation returned by destination reservation — is recorded by the producer
before commit and is the sole replay authority. No `instances` means a
singleton inside the enclosing Graph instance.

The key type is inferred after total port resolution and transported in the
validated plan. It is either a canonical map-key value or a recursively closed
struct of canonical key leaves. Struct field order is erased by canonical
name ordering; field names, resolved types, and values remain identity.
Missing/optional fields, float, `number`, null, interval, variant, unions,
collections, tables, and opaque values refuse.

Ordinary Nodes check that expression under every resolved admitting input.
An inbound source instead uses its contract's resolved ingress-produced
envelope type as the same admission signature; a source without a graph input
never gains an untyped key path.

**Address is hierarchical; placement is not identity.** A logical address is
the static owner path followed by dynamic keyed/time-box frames, outermost
first; singleton ownership adds no frame. A later delivery may create a fresh
instance at the same address after closure. The runtime's zero-based
generation is derived from journaled `l:start` facts, fences stale
observations, and never enters the logical address, call identity, or source
vocabulary. Routes and transactions carry the instance reference `(address,
generation)` so delayed physical delivery cannot cross a close.

Every keyed owner is a routing boundary and may, by default, be assigned to a
virtual shard independently of a keyed parent. Cross-boundary traffic remains
an ordinary journaled delivery even when colocated. Physical colocation,
Worker count, Host, lease, and leader epoch never change source semantics or
causal identity. A stable virtual-shard count is independent of Worker count;
changing it is an explicit reshard migration.

**Mutable state has one Node owner.** A `var` is visible only in the handlers
of the Node that declares it. It is never inherited by child Nodes and no
Graph traversal is an implicit transaction. A child may declare the same
spelling as an independent cell. Graph-level immutable `val` bindings remain
lexical and may be used by descendants. Shared mutable behavior is an
explicit stateful Operator reached through typed ports. This retains the node
transaction as the only ACID unit and makes independent placement possible
without remote shared memory.

**Immutable context has an explicit phase.** Unit/App `val`s are boot
bindings. A `val` inside another Node is derived once per Node instance from
enclosing immutable bindings and that owner's local `instance.key`; it cannot
read the entering payload or mutable state. Graph values are lexical to
descendants, while Operator/Connector values are private. Connector contracts
declare the latest permitted evaluation site — plan, boot, instance, or
invocation — and checking selects the earliest site where the expression
closes. Anything selecting a profile, port type, or topology fact permits only
plan resolution. Instance-capable configuration may use enclosing `val`s and
the Connector's own key and is evaluated once into the instance bundle; a
literal may still reduce at plan and an `env()`-only value at boot. The
categorical plan transports the typed expression and its actual phase; an
adapter never reinterprets a plan-resolved choice as dynamic.

Nested entry proceeds outermost to innermost. Each owner key is selected and
its candidate instance bindings are validated before a child may consume
those bindings. A source Connector's bootstrap fields must close by boot;
instance-site configuration exists only after an accepted envelope selects
the instance and cannot configure the listener or poller that accepted it.

**Compilation preserves the owner tree.** Spatial cycle checking may expand
composition into one flat DAG, but lowering may not erase a Graph that owns
ports, immutable bindings, state, handlers, `instances`, or `lifetime`. The categorical plan
materializes Graph, Operator, and Connector owners under one parent relation.
Each route retains the lexical Graph that owns the edge, so crossing a keyed
Graph boundary is derivable and replayable even when the executable edge is
otherwise flattened. This is required by hierarchical addresses and by
future independent placement; a path string alone is not a lifecycle tree.

**Time is a separate common attribute.** `lifetime` is orthogonal to
`instances`: omission means persistent; `idle`, `fixed`, `single_pass`, and
processing-time `tumbling` use one composite attribute family. A lifetime
without a key is a real temporal singleton within its enclosing Graph address,
and is App-global only when all ancestors are singleton. Idle measures mailbox
admission silence; its expiry requests Closing even when obligations remain,
and the separate bounded closing protocol must settle them without erasing a
Connector exit or Saga compensation. Sliding windows remain deferred because
one admission would select several instances.

**Containment owns lifecycle, not memory.** Bare `stop` targets the current
Node instance. Stopping a Graph also requests inner-first closure of its live
descendant instances. Child start/stop relations are runtime journal facts,
not application state. A non-root child's `l:start` carries its parent
instance reference because ancestor generations are deliberately absent from
the logical address; the fold can therefore support the same cascade across
future shards. The R5 amendment below freezes the finite `closing` sheet,
drain, cancellation, settlement, successor-admission, and residual-delivery
rules. No hidden default is authorized.

**One correlation model.** A join correlates within its owning Operator's
logical instance. `where` refines candidates and `within`/`last` bound them;
the former `by` override is removed. A distinct correlation domain is a
distinct keyed owner.

**Named supersessions.** This decision supersedes D-2026-08-16-1 and the
unreleased P0–P3 source model: `partition` declarations, inline partitions,
all `per` forms, standard partitions, `<partition>.key`, nominal
cross-partition reads, `stop <partition>`, shared Graph `var`, and join `by`.
It preserves P0–P3's canonical key bytes, pre-commit route decision,
journaled replay authority, keyed mailbox/state/open-call isolation, causal
start/stop, generation fences, and one-worker reference implementation. The
unpublished v8 draft is categorically refused by v9; no deployed compatibility
promise attaches to its old `PartitionSpec` spelling.

This rebase declares two separate identity consequences. The categorical plan
body moves once when the new owner/instance tables land; semantic graph hashes
move only when the semantic plan changes. The causal trace format moves for
every firing when `(address, generation)` becomes mandatory journal authority,
including for singleton owners. Those R3 trace and golden moves must be
enumerated before they are re-blessed; they are not cleanup noise.

**R2 transport checkpoint (2026-08-21).** Plan-package v9 is the categorical
owner transport and categorically refuses the unreleased v8 named-partition
body. The body now has two deliberately distinct, reciprocally validated
tables: every Graph, Operator, and Connector has one lexical `Owner` row,
while only executable Operators and Connectors have a `Node` row and a VM
`Slot`. The root App Graph is the unique parentless owner; every other owner
names a preceding enclosing Graph, every Node points back to its exact owner,
and every edge names its lexical Graph scope. Keeping these two domains
separate preserves executable-slot stability without allowing spatial DAG
flattening to erase lexical or lifecycle ownership.

Each owner carries one explicit `InstanceSpec`: an optional checked key tree
whose resolved `DataType` travels with the expression, plus one of the closed
`persistent`, `idle`, `fixed`, `single_pass`, or `tumbling` lifetime forms.
Owners also carry immutable typed expressions with their derived earliest
evaluation site and, exactly at plan site, their independently reproducible
reduction. Effective Connector attributes gain the corresponding dynamic
expression row. Registry-free validation proves the owner tree and reciprocal
links, key domain and input-wide type agreement, positive duration budgets,
lexical binding availability, acyclic value dependencies, exact evaluation
sites, and exact plan reductions. The active `Process` mirror additionally
prevents any dynamic row from changing a plan-shaping attribute without the
compiler Registry; a dynamic contract row with no core mirror refuses closed.

This is intentionally a **package-only** identity move for existing programs.
The `echo`, `gauge`, and `jsonl` package goldens and the embedded spike copy
move because v9 always carries the root owner and owner references. Their
semantic hashes and all causal traces remain byte-identical. R2 is not thereby
declared complete: source production of non-plan owner values and dynamic
Connector settings, nested Graph lowering, and the mandatory canonical
`(address, generation)` route/journal sub-forms remain their named gates. The
last item is still the separately declared R3 causal-trace move; it must not be
smuggled into this package rebaseline.

**R2 validation hardening (2026-08-21).** A closed transport vocabulary is
not an execution promise. V9 can parse every allocated lifetime discriminant,
but `ValidatedExecutionPlan` currently admits only `persistent` and `idle`;
the checker names `fixed`, `single_pass`, and `tumbling` as R4 deferrals and a
foreign plan receives the matching typed refusal. Graph-owned key/lifetime
routing likewise refuses until R3 instead of running as an inert singleton.

The key result type is a derived fact, not a normalizable package claim. The
source lowering boundary may close its single data-input inference variable
from the total resolved port table. Every freely constructed or decoded plan
must already carry exactly the type re-derived from its expression, resolved
inputs, the transported metadata schemas, and enclosing immutable values; a
mismatch refuses rather than being overwritten. Before finite R5 Closing,
an `idle` Process owner additionally requires a positive plan-resolved call
`timeout`: otherwise one open invocation could turn expiry into an unbounded
implicit grace period, which this decision explicitly forbids. Idle dated
events in the R2 VM are fenced by their causal last-admission id; they do not
fabricate the journal-derived instance generation that R3 must introduce.

**R3 routing checkpoint (2026-08-21).** The runtime now has one canonical
`InstanceAddress`: canonical CBOR over the static owner path plus ordered
outer-to-inner selected-key frames, each sealing the resolved `DataType` and
canonical `Value`. `InstanceRef` adds the zero-based generation derived by the
journal's `l:start`/`l:stop` fold. The address alone is the input to placement:
the virtual-shard proof seam is `u64::from_be_bytes(blake3(address)[0..8]) mod
virtual_shard_count`; generation, Worker count, Host, and scheduler order do
not participate.

Ingress is now a prepare/commit operation. Preparation mints the final root
envelope, including `meta.sys`, without consuming its sequence; the VM then
selects and validates the complete destination owner chain outermost first,
checks mailbox capacity, and only then commits the inbound fact and its route.
A normal transaction follows the same select/check/commit discipline. Thus a
key may honestly read the complete immutable envelope, and failure of a key,
owner value, or instance-bound Connector setting commits no ingress,
`l:start`, cursor, state, or external effect.

Every routed delivery now seals a full target instance reference, including
persistent singletons. Every transaction seals its firing reference; consumed
cursors use `(edge, source_instance_ref, seq)`; mailboxes, Node-local state,
pending calls, delivery budgets, lifecycle work, and idle events use that same
reference. Lexical edge scope and the validated owner tree decide which
ancestor frames are retained and which child owners are entered; dotted paths
are never parsed to reconstruct containment. A newly entered child records
its enclosing Graph reference on `l:start` only. Delayed routes retain the
generation they committed, while a later admission after a journaled stop
receives the next ordinal and clean state.

The causal-trace move declared above is therefore made now, separately from
plan/package and semantic identity. `gauge-roundtrip.trace`,
`echo-roundtrip.trace`, `jsonl-roundtrip.trace`, and `m0-probe.trace` move:
INBOUND records carry key 25 route references, every TRANSACTION carries key
31, consumed positions carry the full source reference at key 33, emitted
routes carry full target references at key 38, and nested `l:start` may carry
key 48. No semantic hash or `.rkp` body moves. The connector-free `gauge`
hash remains the scope witness.

**R3 review closure (2026-08-21).** The owner path in the address and in
every frame is sealed as an array of UTF-8 path segments, never as dotted
display text. This makes segment boundaries structural identity instead of
depending forever on the current lexical exclusion of `.` from a segment.
The first, unreleased R3 address spelling is superseded. This correction moves
the same four causal trace fixtures once more; semantic hashes and `.rkp`
bodies remain unchanged.

At the R3 checkpoint, the one-worker VM executed Graph-owned persistent
routing, nested Graph/Node addresses, sticky ancestor selection across
metadata rewrites, isolated state/cursors, journal-derived fresh generations,
pre-effect instance bundles, and the architecture-neutral shard seam. Fixed,
single-pass, tumbling, and Graph lifecycle/cascade still belonged to R4 at
that checkpoint. Source production of non-plan owner values and some dynamic
rows remains an R1 frontend gap; R3 proves the runtime contract with
validated-plan probes rather than pretending that missing source spelling is
already public. Full crash recovery likewise remains an explicit
`RecoveryFold`; the R3 journal rows are its authority and never trigger
re-routing.

**R4 lifetime checkpoint (2026-08-21).** Every selected owner now opens one
lazy, journaled generation, including persistent singleton Graphs and Nodes.
Persistent arms no deadline. Idle is anchored to the most recent boundary
admission; fixed is anchored to `l:start`; single-pass admits exactly one root
envelope and closes at natural quiescence; processing-time tumbling adds the
epoch-aligned box start as an address frame and closes at the box end. A
closed tumbling-box address has no second forward generation: a later attempt
in that same box refuses rather than reopening or spilling into another box.

Every dated lifetime event carries the complete `InstanceRef`. Its sealed
timer reference derives under domain `0x21` from canonical address bytes and
the little-endian generation. Idle also carries the last-admission id, while
fixed and tumbling carry their creating cause. A stale event therefore cannot
address a successor generation or win against newer idle activity. Equal-date
events retain scheduler insertion order and lifecycle stops are selected by
canonical inner-first instance order, so two executions record one order.

Finite expiry closes external admission immediately. Work already causal
inside the same Graph closing scope, including a previously admitted
Connector completion, may finish through existing descendants. Before R5
adds explicit drain/cancel/settle budgets, closure waits for natural
quiescence and never invents a kill or grace period. To keep that interim
wait bounded, every `Process` anywhere in a finite owner's subtree must carry
a positive plan-resolved invocation `timeout`; validation proves this for
foreign packages as well as checked source.

Graph closure requests every live descendant and commits `l:stop`
inner-first. The journal independently refuses a child start under an
inactive parent and a parent stop while a child lifecycle remains active, so
the ordering is a replayable fold rather than VM choreography. State is
reclaimed only after the stop fact commits; re-entry at the same logical
address receives the next journal generation and a clean Node-local cell.
Graph-level lifecycle handlers remain the named R1 frontend/lowering gap, and
the exact bounded `closing` sheet remains R5.

R4 deliberately moves causal traces once more without moving source identity
or plan transport. Transaction key 30 is now the lifecycle/transaction
`owner_path`: ordinary Node firings retain their previous text, while Graph
`l:start`/`l:stop` facts can use a lexical Graph path (the App root is the
empty path). Universal lazy starts add lifecycle records to
`gauge-roundtrip.trace`, `echo-roundtrip.trace`, `jsonl-roundtrip.trace`, and
`m0-probe.trace`. No semantic hash or `.rkp` body changes.

The Cortex-M33 spike now identifies the measured relay transaction by its
journal actor/trigger so the new lifecycle turns are executed without being
misreported as relay samples. The unchanged 927-byte package decodes in
15 345 ticks; 32 exact relay samples average 2 767 ticks (maximum 2 979), the
allocator-tracked peak is 104 124 bytes, and `.text + .rodata` is 462 516
bytes. All four written thresholds remain green; the relay margin is reported
honestly as 1.007×.

**R5 bounded-Closing amendment (2026-08-21).** The complete source sheet is
`closing = { drain_for = <positive constant duration>; settle_for = <positive
constant duration> }`. Both fields are required. There is no action field and
no default. Every finite owner whose subtree contains a `Process` requires the
sheet; another owner may omit it and close at natural quiescence. A persistent
owner may declare it to govern bare `stop`. `Process.timeout` remains the
orthogonal per-call budget and is no longer a substitute for bounded instance
closure.

The first causal close request is a committed `l:closing` transaction carrying
the root `InstanceRef` of the closing scope and the two dated obligations.
That journal fact, not a VM flag, closes admission to the current generation.
For a Graph it covers all currently live descendants, and its two budgets clip
the entire subtree; child policies apply only when a child closes
independently. Existing descendants finish inner-first. New external routes
reserve the exact successor generation and wait in its ordinary bounded
mailbox; no second queue or speculative generation counter exists. A route
already sealed to the old generation remains old-generation drain work.

During `drain_for`, admitted internal deliveries and Connector completions may
run. At the deadline, remaining internal deliveries become residual cursor
consumption of the eventual stop transaction and open calls are cancelled in
canonical `CallRef` order. During the following `settle_for`, a confirmed
completion wins close-once and publishes its ordinary typed `exit`. At the
settlement deadline, every still-open OUTBOUND is atomically closed
`abandoned` and publishes the Connector contract's empty `OutcomeUnknown`
case with trigger `l:recover`. A later reply is a non-causal discarded
observation. A confirmed `Process` cancellation normally yields `Exited`; it
is never rewritten as unknown. `on stop` runs after the final required exit,
then `l:stop` commits, state is reclaimed, and the waiting generation becomes
runnable. Loop wakeups die with their generation; Saga compensation debt
survives as a journal fold.

Bare `stop` is one ordinary statement that requests this protocol for the
current owner generation after its handler transaction commits. It has no
target operand and is rejected recursively inside `on stop`. The plan carries
it as an ordinary step and transports optional Closing at owner-instance map
key 3 under categorical v9. The causal record gains transaction key 49
`close_scope_ref`; timer kinds 6 and 7 name drain and settlement, and domain
`0x22` derives their distinct references. These are extend-only additions.

Adding empty `OutcomeUnknown` to `ProcessExit` is a deliberate semantic
identity move for every Process graph: compiler probe `29c69b70…`, JSONL
`a058f1aa…`, and echo/spike `419a52a1…`. The connector-free gauge remains
`0d3ad43d…`, proving the scope. Their v9 package bodies move with the exact
union; existing non-closing causal traces do not move. A source that writes an
explicit `closing` policy additionally hashes its two expression trees under
the `rastok.closing.r1` extension; omission cannot collide with an explicit
policy.

**R6 composition, joins, and public-evidence checkpoint (2026-08-22).** A
nested Graph is now a public composition boundary from source through the
checker, HIR, lowering, categorical v9 validation, package round-trip, and VM.
Its lexical owner row, boundary declarations, immutable `val` scope, instance
policy, and lifecycle survive lowering; only its passive routing seams are
flattened to executable edges. The resulting flat graph is checked again for
cycles at the Registry-free plan boundary. A cycle hidden by composition can
therefore neither disappear in lowering nor enter `ValidatedExecutionPlan`.

Join activation is the closed v9 handler form `[3, ports, within?, last?]`.
The declared port order is semantic: it fixes named bindings, tuple identity,
and transaction key 34. Inside one owning Operator instance, the arrival that
first completes presence is the trigger; candidate tuples are tried in
declared-port order with FIFO choices, and a false `where` retains every
candidate. `last` evicts the oldest excess candidate per port and `within`
arms one dated obligation per retained delivery. Both produce the ordinary
empty `JoinExpired` tagged case on the v1-required `expired` output; neither
drop is silent. `having`, event-time correlation, and a second key authority
remain unavailable. Bare `data` and `out = .` have no referent in a join;
`a.data`/`a.meta` name one complete input envelope, and each payload owns an
independent lazy materialization witness.

Writable metadata follows CommonOrAbsent over present contributions: absent
everywhere stays absent, one distinct present value survives, and conflicting
present values disappear. `content_type` is selected only by the emitted
payload. The R6 implementation supersedes only one transport detail of
D-2026-08-10-2: v9 does **not** duplicate the canonical app/ext schemas into a
travelling `JoinMetaPlan`. The Registry-free validator derives conservative
presence facts from those already-canonical schemas, activation, explicit
writes, and topology; a required bare read after a join refuses until the
handler explicitly chooses a named contribution. Runtime fusion is the final
defense, not the primary presence checker.

Public Lab scenarios now prove canonical-key isolation, nested addresses,
all five lifetimes, clean successor generations, inner-first Graph closure,
instance-local join correlation, typed expiry, and canonical trace keys 31
and 34 without inspecting private state. The bounded-Closing model enumerates
the reply equivalence classes before drain, during settlement, on cancel,
after settlement, and never, proving exactly one terminal OUTBOUND fact and
that `exit` precedes `l:stop` in every case.

R6 changes neither existing semantic hashes, packages, nor causal traces:
the new activation/hash material is present only for newly executable joins
or nested Graphs. The unchanged 961-byte echo package still hashes to
`419a52a1…`. On Cortex-M33 emulation it decodes in 16 864 ticks; 32 ordinary
relay samples average 2 752 ticks and peak at 2 986, with a 106 788-byte heap
watermark. The no-join dispatcher and single-payload evaluation retain their
direct paths, so the established relay bound remains green rather than being
paid for by programs that do not use R6.

## D-2026-08-16-1 — A partition key is a journaled routing decision

> **Superseded in full by D-2026-08-18-1 before public release.** The key and
> route proofs survive under Node-owned hierarchical instance addresses; the
> named `partition` / `per` surface, `meta.sys` exclusion, flat state binding,
> and fused lifetime descriptor do not.

**`per` selects an instance, not a worker.** A keyed Node has one logical
instance for each canonical key. Its mailbox, mutable state, open calls and
lifecycle are indexed by the same `(node, partition_key)` pair. A Node without
`per` has one explicit singleton instance. Transactions are serialized and
FIFO within an instance; scheduling order across keys is deliberately
unspecified because no graph state is shared across those instances.

The reference M1 runtime remains synchronous, single-process and one-worker.
That implementation is the degenerate case of the normative M:N scheduler,
not a new language restriction. A bounded thread pool, work stealing, shard
movement, or the documented Coordinator/child-Worker process model may later
execute independent keys concurrently under the same as-if-RTC and
journal-relative contracts. No async dependency, mutex, worker thread or OS
process enters M1.4 merely to simulate that future topology.

**The key is a Rastok value with one canonical byte authority.** Its admitted
type domain is the canonical map-key domain from `types.md`: closed scalar or
enum types with a total tagged encoding; float, `number`, null, interval,
variant and composites refuse. The runtime key is those tagged canonical
bytes, not text and not a host hash. Composite `per a, b` is a framed tuple of
component key values in written order; its exact frame lands with the
composite-key gate rather than being guessed by the first single-key lift.

**Routing evaluates once; the journal remembers.** When an envelope crosses
an edge into a keyed Node, the target key expression is evaluated against that
envelope before the producer transaction commits and before mailbox capacity
is reserved. The resulting bytes choose the target mailbox. The target
transaction records those same bytes as identity key 31. More importantly,
the causal producer records every non-singleton selection *before delivery*:
INBOUND key 25 stores `[edge, target_partition]`, while TRANSACTION key 38
stores `[delivery_id, edge, target_partition]`. Thus a crash after the source
commit but before the target fires cannot reopen the key expression. The
target's consumed cursor records the producer's partition bytes as key 33's
`[edge, source_partition, seq]`, and the sequence is allocated per
`(edge, source_partition)`. Replay reads the recorded routing decision; it may
re-evaluate the expression as an integrity check, never as the authority that
could reinterpret old traffic under a changed graph.

The admitted key vocabulary is `data`, `meta.app`, `meta.ext` and graph
constants. `meta.sys` is excluded: the runtime mints that zone at the source
commit, while target selection must already be complete before that commit.
This is a causal boundary, not an expression-engine limitation.

An omitted input-port annotation does not force an annotation merely because
`per` reads `data`. The M1.3 port solver first closes every effective input
type; the partition expression is then projected through those resolved
inputs and its one effective canonical key type is materialized in the
validated plan. Different input ports may share the expression only when it
resolves to the same key type for each; otherwise compilation refuses by
name. Thus `annotation omitted != variant` remains true at this boundary too.

This deliberately rejects the alternative “the key is derivable, so do not
journal it”. A key expression is derivable from a loaded plan and envelope,
but the selected destination is an event of that hop, just like arrival order.
`envelope.md` and identity keys 25/31/33/38 assign that fact to the journal.

**Lifecycle is per instance.** The first envelope admitted to the keyed
mailbox lazily starts an incarnation and causally triggers `on start` before
its port transaction. Mailbox admission is the precise activity point. A bare
positive duration is an idle TTL re-armed there with a generation token;
`persistent` arms nothing, and stale generations do nothing. Expiry or an
explicit `stop` closes admission and causally triggers `on stop`. An idle stop
uses the last admitted envelope as its explicit cause (TRANSACTION key 39).
If a Connector call is still open, expiry marks closure pending: the call's
terminal `exit` commits first, and only then may `l:stop` run. State is
detached from the live key at closure and reclaimed only after the stop
transaction commits. An arrival during closure belongs to a following clean
incarnation, never to the detached one. Fixed/window, single-pass and
composite lifetime execution remain named gates; syntax is not partial
permission.

**Call exit is nested, never replaced.** Stopping a partition first closes
admission, then follows an explicitly bounded drain/cancel policy. Every
Connector invocation admitted before that boundary retains its own call
obligation and terminal output (`Process.exit` in the current contract).
Neither `l:stop` nor cancellation may suppress that exit. The exact policy
surface must be decided before the stop/cancel gate; M1.4 will not bury a
process-killing default in the VM.

**Transport and identity.** The resolved partition descriptor — optional
name, compiled key expression and lifetime — is semantic HIR identity. Its
first executable transport is plan-package **v8**. V7 remains categorical and
does not acquire an after-the-fact field. Adding v8 changes package bytes for
all plans but changes a graph's semantic hash only when that graph actually
binds `per`; a partition-free graph is the immobility witness. Canonical
journal keys 31 and 33 were already reserved; keys 25 and 38 extend the
corresponding record tables without renumbering. Empty-key routes remain
omitted because the singleton destination is plan-derivable.

**Rejected:** one thread/task per key; a `String` runtime key; relying on
hash-map or scheduler iteration order; recomputing an unrecorded key during
replay; treating a named partition as shared mutable graph state; cascading
stop through similarly named partitions; and introducing Host/Worker process
isolation as part of this semantic slice.

## D-2026-08-14-1 — Rastok owns payload types; frontends are `name!`

**The owner changes.** Rastok's logical payload type system is the
canonical [`DataType`](types.md) tree. It types envelope data, ports,
state and the admitted metadata subset. The intrinsic expression
language consumes `DataType` and the one materialized `Value` domain; it
does not define either. The former `cel::Type` authority has moved to
core `datatype::DataType`, and materialized values now live only in core
`value::Value`. The expression evaluator's `expr::EvalValue` is transient
evaluation state: it may carry the lazy whole-payload witness, but it is
never serialized, journaled, or accepted as an envelope value.
The language is named **Rastok expressions**. Its compatible operator
core remains differentially checked against cel-spec, with every
Rastok-owned divergence named, but “CEL” is no longer the specification
or module-level type authority.

**Static means resolved, not monomorphic.** Source type annotations may
be omitted and inferred; a validated plan nevertheless carries an
effective type for every port. `variant` is the explicit dynamic type,
never the fallback for an unsolved inference variable. A Connector
contract may describe fixed, same-as, profile/attribute-selected or
constrained generic port types. Compilation resolves those rules for the
particular instantiation and ships the complete table; the runtime never
consults the compiler Registry. A codec changes bytes ↔ value
representation. A logical schema change requires a visible typed
Operator/adapter.

**All ports share one model.** A Node has typed input and output ports;
`in`, `out` and `err` are conventional default names, not a second type
or emission mechanism. A Connector additionally uses `exit` by convention
for the terminal outcome of one admitted invocation. It remains an ordinary
typed output: the convention gives it lifecycle meaning, not another
emission or conformance path. Tagged-union construction is a general value
form and every assignment to a port lowers through the ordinary emit path.
The connector-level `error_set`, `TypedError`, failure-only `CallOutcome`
branch and error-specific `err_value` grammar were temporary approximations,
not a model to extend. B5–B7 have removed all four from executable code.

An Operator has no mandatory `err` port and the runtime never synthesizes
one. Its effective table contains only ports declared, handled, emitted or
wired by the program. A simple Operator with `in: I`, `out: R` is analogous
to `I -> R`; adding an explicit `err: E` gives the useful `I -> Result<R,E>`
shape, without making `err` exceptional or imposing mutual exclusion on a
general multi-output handler. `E` is a domain result deliberately emitted by
the graph. A failed expression, impossible materialization, invalid plan
reference or other engine fault aborts the node transaction and reaches the
supervision boundary; it never becomes an envelope on `err`. If a dynamic
condition is an expected business alternative, the author checks it (for
example with `is`) and emits the chosen domain value explicitly.

This generalizes rather than abandons the original M1.3 typed-error goal.
Its error codes become cases of the invocation's tagged `exit` union, beside
non-error terminal cases. `err` remains diagnostic data actually observed at
the boundary; an error case is never fabricated as stderr, and bytes on
stderr never classify an invocation as failed by themselves.

For `Process(job)`, the intended port contract is exact:

```text
in      -> stdin, framed by the selected request codec
out     <- stdout, interpreted by the selected reply codec
err     <- exact stderr bytes
exit    <- one tagged ProcessExit lifecycle/outcome value
```

An observed non-zero exit is still `Exited { exit_code, signal }`; it is
a fact, not an automatically classified application failure. Spawn,
timeout, capture, output-budget/capacity and decode outcomes are cases of
`exit`, never invented stderr. Every admitted invocation creates one call
obligation and completes as:

```text
out* + err* + exactly one exit
```

The terminal `exit` is a causal barrier for that invocation: no later `out`
or `err` may belong to it. One boundary completion is the control fact that
closes the obligation and may carry several ordered typed port outputs; the
VM alone turns those drafts into envelopes and journal emissions. Call
identity correlates every output, so two concurrent invocations in the same
Node instance cannot mix their frames or buffers. Call closure is therefore not
inferred from a port name, even though the resolved Connector contract names
its terminal output `exit`.

**`stop` and `exit` terminate different scopes.** A Node definition is
static. At runtime it may have several logical instances in parallel, and one
Node instance may admit several Connector invocations. `stop` requests the
end of the current Node instance; `exit` reports the observed end of one
invocation. Many `exit` values may therefore precede one Node-instance
`stop`. Closing an instance must stop admission and either drain or cancel
its open invocations according to an explicit bounded policy; it may not
erase their obligations. Each admitted invocation still produces exactly
one `exit` before the instance is reclaimed. M1.3 fixes and implements the
invocation contract for `Process(job)`; M1.4 implements Node-owned instance
addressing and bounded Closing without changing that port model.

Framing and publication are separate decisions. A line framer may yield two
hundred `err` envelopes for one invocation, while a bounded collector may
publish them only at completion or aggregate them under an explicit policy.
`Process(job)` defaults to an on-exit completion so retries cannot leak a
partial attempt. A future streaming profile keeps the same invocation
identity and terminal barrier. Aggregation is always bounded; overflow is an
`exit` case rather than a truncated value presented as complete.

`Process.codec` frames `in` and `out`; the independent
`Process.err_codec` frames stderr and defaults to `lines`. A `jsonl` data
codec therefore never tries to decode diagnostics as JSON. Framing produces
unit values (`lines` → one `text` per line, `jsonl` → one `variant` per
document, `bytes` → one bounded captured byte value). On-exit publication
commits those ordered values atomically with `exit`; it does not silently
fold them into an array or one text blob.

B5 introduced `err_codec` into the executable Process sheet and materialized
its default before hashing like every other contract default. That deliberate
identity move was inventoried and re-blessed in B5; B7 separately inventories
the Process framing/completion move rather than conflating the two causes.

The removal of the temporary failure scaffold follows this explicit table;
an implementation never maps by similar spelling:

| Temporary Process fact | M1.3 completion fact |
|---|---|
| successful `CallOutcome::Success` | zero or more decoded `out` frames plus `Exited { exit_code = 0, signal = null }` |
| `SpawnFailed { message }` | `exit = SpawnFailed { message }` |
| `TimeoutExpired` | `exit = TimeoutExpired` |
| `DecodeError { message }` | `exit = DecodeFailed { message, line = null }` (the new framer may supply the optional offending line) |
| `CaptureFailed` | `exit = CaptureFailed` |
| `OutputOverflow { limit, reason }` | `exit = OutputOverflow { limit, reason }` |
| `ExecFailed { exit_code, stderr }` | the observed child status becomes ordinary `Exited { exit_code, signal }`; stderr is independently framed into `err*` and never remains in the exit payload |
| declared `StderrLine` | no exit case; every observed stderr frame is ordinary data on `err` |

In particular, a non-zero process code changes from an adapter-classified
failure into an observed terminal fact. Retry or alert policy belongs to the
graph after narrowing `exit`, not to the Process adapter.

**One marked frontend form.** A value region owned by a dedicated
reader/compiler is introduced by:

```text
tagged_value ::= IDENT "!" frontend_args? frontend_payload
```

Examples are `json! ...`, `text! { ... }`,
`base64!(content_type) { ... }`, `jq!(data) { ... }`,
`sql!(data) { ... }` and an application-registered
`starlark!(data) { ... }`. The postfix bang is host syntax, distinct
from the expression language's prefix negation and `!=`. It lets the
left-factored parser select the frontend production with one token of
lookahead, without a keyword set or rewind. Built-ins and registered
frontends use the same surface. The selected frontend still owns its
framing; the sigil identifies the owner but cannot teach the host a
guest's comments and strings.

The name and bang are byte-adjacent; intervening trivia refuses. A frontend
fills one complete host value and is not an intrinsic-expression atom, so a
frontend result is bound before being combined with expression operators.
Lexical maximal munch gives `!=` priority over the single bang:
`json!=x` is `json != x`, not a frontend.

Frontend names are consequently ordinary bindable identifiers. `json`
is no longer reserved. `nothing` remains a host structural word.
`asset(...)`, `duration(...)` and `decimal(...)` remain ordinary Rastok
expression constructors because they consume arguments rather than a
foreign source region.

**Full JSON values return.** The unambiguous `json!` reader accepts
object, array, string, number, boolean and null. Every result has static
type `variant`; an untagged Rastok scalar keeps its static scalar type.
The house reader consumes exactly one JSON value, refuses trailing
non-trivia, duplicate keys and out-of-budget exact numbers, and never
silently narrows to double.

**External schemas project into the same algebra.** The canonical type
tree covers the supported JSON Schema, Avro and Arrow profiles without a
lossy `bytes`/`variant` escape: widths and signedness, decimal/temporal
parameters, records/structs, maps, arrays/lists, enums, fixed binary,
nullability and unions. Avro branch identity and Arrow union type codes
become stable logical discriminants when structure alone is
insufficient; container indexes and dense/sparse layout remain codec
facts. A JSON `variant` may be checked or narrowed to any declared union:
exactly one structural branch must match unless a discriminator selects
a tagged case. Zero or multiple matches refuse. Unsupported recursive or
extension schemas refuse at their exact path rather than degrading the
type. The core stays independent of Avro/Arrow libraries; boundary
adapters own the projections.

**Versions remain separate.** The M1.3 plan/package change is one
categorical format v7 landing; v6 packages are refused and no backward
reader constrains the new canonical type registry. That does not remove
application schema evolution. `graph_version` continues to select
retained historical schemas from the manifest. A V1→V2 adapter consumes
the old typed envelope and emits a new one with normal lineage; it never
rewrites history. Journal migration onto a different semantic hash stays
deferred and refuses typed until that explicit execution contract lands.

**Classifications wait beside types.** A future marker such as
`@sensitive` is schema policy, not a new logical text/struct type. It may
later affect propagation, sinks, redaction, retention, telemetry and
plan identity. This decision frees `@` from the proposed custom-guest
sigil but neither allocates it nor serializes placeholder annotation
tables in M1.3.

**One landing, not a public intermediate.** M1.3 replaces the former
typed-error item and follows the ordered backlog in
[`m1.3-type-system-backlog.md`](../architecture/m1.3-type-system-backlog.md).
Intermediate commits may use private adapters to keep the suite green;
no transitory `error_set`/kind-list architecture becomes normative or a
published package format. If schedule pressure appears, later M1 breadth
is reduced before this foundation.

**B3 implementation checkpoint (2026-08-14).** The code boundary is now
`expr`; the old `cel` module/API no longer exists. Signatures and trees consume
core `DataType`/`Value`, and the evaluator-only carrier cannot cross a
materialized boundary. `is`/`as` share one typed narrowing judge: structural
union selection is exactly-one and exposes only its selected value to later
expression operators while retaining the branch index in materialized data;
tagged selection preserves the name, external discriminators are explicit,
and mismatch, ambiguity and invalid
configuration remain distinct. Positive tests narrow only the right side of
`&&`, true ternary branch and guarded handler body. At this checkpoint the
private v6 validator refused these new AST forms with
`ExpressionNotInFormat`; no private tag was added.
The compatible expression subset is independently evaluated against
exact-pinned, dev-only `cel-interpreter 0.10.0`; all divergences are recorded
in `expressions.md`. B8 subsequently removed the private-v6
`decimal(...) -> variant` bridge: the v7 tree type is canonical `number`.
B9 subsequently removed unchecked member/index access on `variant` and the
historical format-refusal scaffold itself.

**B1 completion checkpoint (2026-08-14).** A materialized `Value` carries
canonical data, not a duplicate `DataType` tree. Integer/float width, fixed
length, decimal precision, temporal unit and timezone are resolved from the
port/schema selected by `graph_version`; enum symbols and union selectors stay
in the value because they select semantic data. The core value/conformance
matrix now covers every family in the strict JSON Schema, Avro and Arrow
profiles, including exact binary16/32 promotion, UUID, all temporal shapes,
canonical non-text map keys and Avro's unsigned month/day/millisecond interval
without conflating it with Arrow's signed intervals. Private v6 refused those
new constant forms; B8 now publishes their typed v7 seal.

**B3 narrowing closure (2026-08-15).** Narrowing an exact integer or decimal
to `decimal(p,s)` rescales to the declared scale exactly: zeros may be padded,
only zero digits may be removed, and overflow/precision loss refuses without
rounding. A structural-union branch index belongs to its exact resolved
union. Ordinary union-to-union assignability therefore requires identical
types; explicit `as OtherUnion` performs a fresh exactly-one selection and
records the target-relative index. Contextual construction or emission into
a union is inserted once by check/lowering, while the VM and adapters only
validate the already selected discriminant.

**B4 implementation checkpoint (2026-08-15).** Byte-adjacent `IDENT!` is the
only tagged-frontend production. The parser selects it without a frontend-name
table or rewind, then consults the compiler Registry for the complete framing
and reader contract. JSON, text, base64, deferred guests and reserved data
formats share that door; frontend names remain ordinary bindings and maximal
munch keeps `json!=x` an intrinsic expression. The JSON reader accepts its
complete value grammar, including scalars, and reports trailing input at the
host position boundary. `asset(path, content_type?)` is an ordinary expression
call whose filesystem binding is resolved before HIR identity or plan
validation; an unbound call is categorically refused by the plan gate. No
plan-format tag, valid-program hash or golden moved in this checkpoint.

**B5 implementation checkpoint (2026-08-15).** `ConnectorContract` now owns
one `PortContract` table: direction, unwired policy and a fixed, same-as,
common-choice-selected or profile-selector-selected logical type rule. A
profile-local attribute cannot select a port type because sibling profiles may
reuse that attribute name with different domains. Resolution happens after
defaults/profile selection and materializes every Connector port as an
ordinary typed plan row. The package validator then checks the complete active
`Process` table without the compiler Registry. Inputs cannot carry output sink
policy, boundary ports cannot carry node sink policy, and the VM routes an
unclaimed output only from the row's `RootSink` value — never from the string
`err`.

The B5 executable v6 bridge was deliberately exact rather than aspirational:
`lines` resolved `in/out` to `text`, `jsonl` resolved them to
`variant/array<variant>`, and `bytes` resolved them to `bytes`; the private
success/failure Partner seam still required `err/exit: variant`. B7 replaced
those carriers with framed stderr and repeated unit emissions plus exact
`ProcessExit` at the in-memory completion gate. The B5 sheet nevertheless owns
the complete target vocabulary now: `codec = bytes`, independent
`err_codec = lines | bytes` with materialized default `lines`, the conventional
`exit` row, and the fixed old-fact-to-`ProcessExit` migration table.

The travelling `error_set` is removed. Its private-v6 connector key is retired,
and the port row's direction code now also carries `RequireWire | RootSink`.
These are deliberate pre-public identity/package moves: echo/spike
`c75b840d… -> a7511135…`, JSONL `60f45914… -> 4eacb55c…`, and the compiler
probe `e26af508… -> f068bcd1…`. The B5 implementation also made the former
implicit Operator runtime-fault row explicit in private package bodies. The
M1.3 closure below supersedes that intermediate choice: an Operator has no
such producer or row. Causal trace goldens remain unchanged.
The unreleased v6 reader refuses prior bodies at validation or canonical
re-encoding rather than migrating them. B8 subsequently replaced the body with
the single published v7 type/expression registry.

**B6 implementation checkpoint (2026-08-15).** `IDENT { ... }` in a host
value position is the predictive tagged-constructor production. The parser
records the case and closed body without guessing a type; the checker resolves
exactly one compatible tagged union from the destination or the unit's named
types. Missing, extra, repeated or incompatible fields refuse at that door.
An obsolete D-2026-08-12-1 initializer name that resolves to no genuine case
keeps its migration diagnostic; declaration/configuration sub-blocks remain
dead forms. Thus the old rule is superseded only where type context makes the
same shape semantic data.

Contextual construction into a structural union is represented explicitly in
the checked expression tree as the same typed `As` operation used by explicit
narrowing. Exactly one branch must accept the source type; no match or several
matches refuse `E-PORT-TYPE`. Nested case fields use the same door, so an
optional `null` is wrapped before the tagged payload is materialized. A value
already carrying a different structural-union type never takes that implicit
door: source-written `as Target` must reselect the target-relative index.
Runtime evaluation executes the resolved node and the ordinary conformance
gate verifies the result; neither runtime nor an adapter selects, repairs or
reindexes it.

The syntax and HIR `ErrEmit`/`err_value` split are removed. `err = value` is an
ordinary assignment to an ordinary output and lowers through
`Statement::Emit`, exactly like `out`, `exit` or an application-named port.
Only its resolved `RootSink` policy is conventional. Struct/tagged constructor
and contextual `As` nodes remained categorically outside v6 package format;
B8 now publishes them once with the complete v7 expression/type registry.
Existing v6 programs, packages, hashes and goldens therefore did not move in B6.

**B7 implementation checkpoint (2026-08-15).** A boundary `Completion`
carries an ordered sequence of ordinary `PortOutput` drafts and structurally
requires exactly one final `exit`. `CompletionGate` validates every draft
against the resolved output pin and validates the terminal materialized value
against exact `ProcessExit` before the journal can close the call. The VM alone
mints envelopes and commits the complete sequence atomically; the Partner and
Lab mocks manufacture neither envelope identities nor journal records.

`Process(job)` now frames stdout and stderr independently and publishes
`out*`, `err*`, then `exit`. `lines`, `jsonl` and `bytes` each produce unit
values; `err_codec = lines | bytes` never inherits the data codec. Every
observed child status becomes `Exited { exit_code, signal }`, including a
non-zero code. Spawn, timeout, decode, capture and output-overflow facts map to
their declared `ProcessExit` cases; stderr is never folded into one. The old
`CallOutcome`/`TypedError` seam is deleted.

At the B7 checkpoint the package remained private v6. Its raw `Process.exit`
row was still `variant` because v6 could not encode tagged-union types, while
compiler, binder, completion gate and runtime used the exact union. B8 has now
removed that sole remaining bridge. B7 deliberately moved Process identities and causal
goldens: echo/spike `e322906a…`, JSONL `87d13844…`, compiler probe
`77d326e3…`; connector-free gauge remains `0d3ad43d…`.

**B8 implementation checkpoint (2026-08-15).** Format v7 publishes one
complete, extend-only `DataType` constructor registry for every scalar
parameter, closed composite and both union families. Struct fields, enum
symbols, tagged cases and resolved-port rows are sorted by their semantic
keys; structural branches use the canonical type order. Constructors and the
package reader enforce the existing member/node/nesting and byte budgets,
reject duplicates and overlapping unions, and never admit recursion or an
unresolved inference variable.

The body carries the mandatory total `ResolvedPortTable`, including each
boundary/node port's direction, exact effective type and
`Declared | Inferred` origin. Decode first parses the claim, rebuilds the
table from the raw plan constraints without a compiler Registry, and requires
exact equality before accepting the package. The expression alphabet now
transports closed structs, tagged constructors, `is`, `as`, their resolved
targets and explicit external discriminators. All B1 materialized-value tags
remain the one registry in `identity.md`; v7 activates interval tag 24 and
both structural-index and tagged-name selectors under union tag 21 without
renumbering an existing value tag.

The B3/B7 representation bridges are gone: `decimal()` carries static
`number`, `Process.exit` carries exact `ProcessExit`, and integer literals
outside the `int32` range carry `int64` (same-carrier arithmetic widens to the
larger declared width). V6 is refused categorically rather than migrated.
Avro branch indexes and Arrow union type codes are adapter mappings to the
canonical branch/case; Arrow dense/sparse and Avro container layout never enter
logical identity. This is a deliberate pre-public identity move: echo/spike
`e322906a… -> 514d4ab7…`, JSONL `87d13844… -> 18dda78c…`, and compiler
probe `77d326e3… -> fa5abd79…`; the connector-free gauge remains
`0d3ad43d…`. Package goldens and the embedded measurement are re-blessed only
for this categorical publication.

**B9 implementation checkpoint (2026-08-15).** External schema projection is
one strict standard-layer boundary, `rastok::schema_projection`, over the
core `DataType`. JSON Schema 2020-12, Avro 1.12 and Arrow V1 adapters either
produce an exact logical type or return a stable loss reason at the offending
path. Reverse projection obeys the same rule. Avro branch indexes and Arrow
type codes are returned as explicit adapter mappings; declaration/layout
order never enters logical identity.

“A bounded integer range fits a width” is not sufficient for an exact
projection: `0..10` becoming `uint8` would admit values the source forbids.
The executable JSON Schema profile therefore accepts only inclusive bounds
equal to one complete canonical signed/unsigned width. Likewise, a JSON
Schema property that may be missing cannot become an always-present struct
field or `T?` (missing is not null); without an explicit boundary fill policy
it refuses as `LossyProjection`. A homogeneous map likewise cannot preserve
the claim that one particular key is required. Avro `fixed` decimal storage
must have enough signed bits for its declared precision. Avro union
uniqueness follows the underlying schema type, so logical annotations cannot
disguise duplicate physical branches; valid boolean JSON Schemas remain
outside the strict V1 profile rather than being misreported as malformed.
These clarify the no-weakening rule and supersede the looser wording
previously present in the projection table.

Unchecked member/index access through payload `variant` is removed. A
program must establish a precise type through `is` or `as`; the checker then
applies the same `DataType` conformance used by packages and runtime pins.
The metadata root remains governed by its separate sealed field grid, not by
dynamic payload lookup. The retired `is_v6_wire_type`,
`DataTypeNotInFormat` and `ExpressionNotInFormat` scaffolding is deleted. A
lowering defect that leaves `asset()` unbound now refuses exactly as
`UnboundAssetCall`.

B9 changes no registry number, package format or echo/spike identity
(`514d4ab7…` remains). Two executable fixtures move because they now state
the schema they inspect: the compiler probe `fa5abd79… -> d9b1c956…` pins its
closed operator input, and the JSONL fixture `18dda78c… -> 9e66e62c…`
narrows each unit `variant` frame to its declared JSON-object map before
indexing. The JSONL package golden is re-blessed for that source-semantic
change; its causal trace is unchanged.

**Operator-fault closure (2026-08-15).** The final M1.3 review removes the
last failure-shaped exception: the VM no longer turns a pure Operator's
evaluation fault into `{ kind: "RuntimeFault", message: ... }`, and lowering
no longer invents an `err: map<text,text>` producer row. Explicit `err = E`
remains an ordinary typed domain emission. A runtime fault leaves the
delivery reserved, commits no state, record or envelope, and is returned to
the supervision caller. The regression suite proves both halves: an
explicit tagged `err` is materialized normally, while failed narrowing and
opaque reads produce `VmError::Fault` with no transaction or root emission.

This correction changes the echo, JSONL and gauge package bodies because
they contained the lowering-only row, but changes none of their semantic
hashes or causal traces. That mismatch was the evidence: the row had never
participated in canonical HIR identity and therefore could not honestly be
program semantics. The package goldens and embedded echo package are
re-blessed to the plan the hash actually describes; format v7 itself and all
registry numbers remain unchanged.

**Supersedes:** D-2026-08-13-1's claims that `json` is reserved, JSON
scalars are removed, and CEL is the intrinsic authority; the foreign
literal spelling and `TAG_KW` partition in D-2026-08-12-1; the
`json <json-value>`, `text { ... }`, `base64(...) { ... }` and bare guest
spellings of D-2026-08-10-5/-6; the proposed `@mylang` guest sigil; the
Process/error-set interpretation recorded in D-2026-08-12-1's capture
amendments; and D-2026-08-11-1's reserved `bytes` codec and single
`array<variant>` job reply (each decoded JSONL document is now an ordinary
`variant` emission). It also supersedes D-2026-08-13-1's claim that `cel`
remains the module/type authority and its indefinite “Dyn until a future type
slice” for `decimal`: the logical and transported target is now `number`; B8
removed the named private-v6 bridge. The unreleased diagnostic
spelling `E-CEL-EXPR` becomes `E-EXPR` in the same pre-public B3 migration.
Their still-compatible parser,
exact-number, capture-budget, attribute-contract and framing invariants
survive.

The Operator-fault closure also supersedes D-2026-08-08-3's recoverable
construction rule that converted an Operator value breach into a synthesized
typed value on `err`. Its final fan-out conformance gate and all-or-nothing
transaction rule survive; only the invented graph result is removed.

B5 additionally supersedes only the *downstream representation* clauses of
D-2026-08-08-1/-2 that called the conventional trio implicit in the plan or
VM. Their source convenience, direction and bounded-restatement rules survive;
the checked HIR and runtime plan now materialize the rows.

## D-2026-08-13-1 — A lexeme-stream parser, json documents, and `decimal(…)`

> **Partly superseded by D-2026-08-14-1.** The forward lexeme-stream,
> recovery, trivia, line-bounded string and exact-decimal rules survive.
> The reserved-`json`, object/array-only and CEL-authority clauses do not.

Consolidated after the parser review (the round-by-round history lives
in Git; the drafts each round superseded are named at the end). The
CURRENT rule:

**One owner for the bytes.** The lexer owns the byte cursor AND the
token lookahead; the parser reads complete lexemes through
`peek`/`next`, asks semantic questions, and never touches bytes — no
path can combine a buffered lexeme with a second read of the same
bytes, because both live under one impl. No repositioning primitive
exists: every cursor write is a locally-derived forward offset
inside the lexer method that owns it. The identifier prefix is
left-factored (`value ::= … | IDENT value_after_ident`); a consumed
prefix is never handed back — a CEL continuation is ONE complete
`CEL_EXPR` lexeme the lexer stitches from the immutable source
slice, boundary lexemes surviving (the mirror of the terminator
test, asked of the buffered token). The named finite-byte-lookahead
points — the ONLY places a production choice reads a byte, all
lexer-owned questions returning CLOSED types, never a raw byte:
the distinct head classifications (`statement_head` → Structural |
BareValue | Recovery; `value_head` → Structural | Cel), the path and
assignment probes
(`dot_follows`, `assignment_follows` — `=` alone vs `==` is
decided WITHOUT tokenizing), `tag_opener_on_line` (→ TagOpener),
`string_argument` (a tag argument's StrLit-vs-CEL head, only a
proven double-quoted string is tokenized), `arguments_close`, and
the line questions (`value_missing_on_line`, `line_exhausted`).

**The implicit yield is REMOVED** (review round 7 arbitration —
recommended by the review, aligned with the LL(1) consigne): a
bare expression is NOT a statement. When its head is not already a
live host-statement opener, a well-framed value is consumed by its
real owner (braced/value parser, CEL region, JSON reader, verbatim
reader, or guest framer), discarded, and represented by ONE statement
error hole carrying the migration hint (`out = <value>`, or
`err = Kind`). Its
interior can therefore never become statements or attributes of the
enclosing construct. After an IDENT has failed the assignment
production, the CEL framer may choose only the recovery EXTENT: it
atomically commits a complete continuation as one CEL region or leaves
cursor and lookahead untouched for line-bounded host-lexeme recovery,
preserving the next healthy statement. This does not choose between
valid productions and does not weaken LL(1); normal values and recovery
share the SAME pure `Complete | Incomplete` extent computation, not
parallel scanners. A normal value commits the full complete extent; an
incomplete value in a newline-ended position commits only its first safe
line boundary, so a following statement is never part of the CEL text.
The host merely frames that malformed value: the CEL compiler emits its
ONE syntax diagnostic. An unterminated block comment is the deliberate
lexical exception: the extent ends BEFORE `/*`, and the next host-trivia
read emits `E-LEX-UNTERMINATED-COMMENT` during parsing. The checker may
never run after the resulting missing-closer errors, so deferring that
cause to CEL would make it unreachable. The computation carries expected
closer kinds, so `)` can never close `[`. Ordinary garbage diagnostics
are suppressed by statement recovery, but a structural lexical cause
(unterminated string/comment) found while tokenizing that rejected host
statement is retained instead of being hidden by the generic position
error. Behind `=`, every form takes the ONE value dispatch. An explicit
`yield <value>` sugar may return later; the host lexer keeps only ITS
comparison lexemes (`==` `!=` `<=` `>=` — the simulation expect grammar),
never a copy of CEL's operator vocabulary.

In statement position, `(` selects the multi-emit production
unconditionally; a parenthesized Rastok expression is a value only after `=`. A
malformed multi-emit recovers forward from the failing lexeme to the
statement boundary. It is never distinguished from a parenthesized expression
by tentative parsing, an unbounded syntactic predicate, or rewind.

Ordinary strings are line-bounded in both lexical domains. For a host
`STRING`, any physical CR/LF before its closing double quote is
`E-LEX-UNTERMINATED-STRING` (a preceding backslash does not buy
continuation), and the next line remains available to recovery. The host
Error token used when a single-quoted expression spelling reaches statement
recovery has the same boundary. In an expression value region the framer stops
the incomplete extent before that physical newline and the expression compiler
emits the single expression diagnostic; its own lexer independently
rejects raw line endings too. An escaped `\n` remains ordinary text;
multiline text uses `text { … }`.

**Specialized lexemes, selected before their content.** Sub-languages
are lexical delegations returning complete lexemes or complete
refusals. CEL is selected by the value-position classification; JSON
by the reserved `json` continuation; brace-bodied verbatim/guest
methods consume their own `{` before any body byte can meet the host
vocabulary. Four region kinds, by SEMANTICS, never one universal
framer: the CEL region (`cel_region` — the intrinsic expression
language); the json continuation (`json_tail` — a data reader); the
VERBATIM regions (`verbatim_region` — `text`/`base64`: braces balance,
every other byte is ordinary data, quotes and apostrophes included, no
escapes); and the GUEST regions (`guest_region` —
jq/bash/sql/python/rust: the frozen contract, dual-quote opacity,
awaiting the guest slice's framer arbitration). The host munches
ITS OWN comparison lexemes (`==` `!=` `<=` `>=` — the simulation
expect grammar; never a copy of CEL's vocabulary), and a
single-quoted string is ONE opaque Error lexeme (CEL's spelling,
named as such once). The parser receives distinct CLOSED head
classifications, never a raw byte. Recovery is selected by semantics:
self-framing misplaced values use their real reader; incomplete or
otherwise unframed statements walk HOST LEXEMES into a scratch bag to
a strong boundary (`;`, a depth-zero `}`, or the next line), always
making structural progress; dead/reserved bodies walk host lexemes to
their balanced closer (a closer inside a comment is trivia and never
closes). Scratch diagnostics stay silent except for the structural
lexical cause that determined the loss of a boundary.

**The guest frontier is atomic.** A guest body reaches its
sub-compiler as ONE region lexeme — exact bytes, exact span, no
normalization; each brace-bodied guest method CONSUMES its own opener
(returning a TOTAL Option, no panic), so no content byte can meet
the host vocabulary — the pre-lexed-content state does not exist.
Guest errors come back as typed results remapped by span. CEL
(intrinsic expressions), the `json` reader (data), and registered
guests are three concepts, never one abstraction. The DELIMITATION of
a region by `{ … }`
under the frozen contract (brace balance, dual-quote opacity)
cannot know a guest's comment or string lexicon — the failure
modes are named and EXPOSED in grammar.md with the arbitration
options (authoring contract / fenced delimiters / guest-registered
framers); the host grows no per-guest lexicon.

**Trivia has one authority.** `rastok_core::trivia::comment_at` is
the single recognizer of the comment FACT (span, terminated,
multiline); the host trivia, the probes, the region scans and the
CEL lexer each apply their own POLICY to it. Comments are trivia
inside CEL regions (the CEL lexer accepts `//` per cel-spec and the
host's `/* … */`), inside recoveries (a boundary byte inside a
comment never bounds), and a line's end can only end a value at
depth ZERO of a newline-ended position — a commented spelling
compiles to the same tree, plan, and hash as the plain one,
whatever the expression's shape.

**`json` is a RESERVED whole keyword** — with `nothing`, the
expression literals (`true`/`false`/`null`), and, for nodes, the
envelope surfaces. ONE naming policy refuses them at every live
declaration door (graph, lab, boundary and node ports, nodes,
val/var/let, type names — `E-NAME-RESERVED`, at the declaration); a
user `type` may not spell a builtin type (`is_builtin_type_name` is
the one vocabulary authority); `DECL_KW` words stay
position-decided. `json`'s follower decides TOTALLY: `{`/`[` on the
line open the DOCUMENT (the reader owns its self-delimiting
braces; duplicate keys refuse; numbers exact and value-canonical);
the position's boundary leaves the bare word (a contract may judge
it); ANY other follower is an invalid json form — E-JSON-VALUE with
its migration, recovered by the HOST-bounded walk (a newline is a
strong boundary at a newline-ended position even inside a paren the
garbage opened; a balanced same-line form swallows whole). The
JSON-document recovery follows JSON's own delimiters instead. The
scalar spellings (`json 2`, `json true`, `json null`, `json "x"`)
are REMOVED; `json [1] ≠ [1]` keeps the prefix's meaning (the
sealed `[type, root]` identity of D-2026-08-10-6 is unchanged for
documents); the runtime jsonl codec is untouched.

**`decimal("…")` is CEL's exact-number constructor** — exactness is
a capability of the expression language, not a side effect of a
JSON prefix. The argument is a standard JSON number; the scale is
DECLARED — `decimal("1.50")` is `{unscaled 150, scale 2}`, trailing
zeros significant, the full decimal128 domain (38 significant
digits, scale ∈ [−6143, +6144], SQL NUMERIC semantics). The
D-2026-08-10-7 integer fold applies to the WRITTEN scale
(`decimal("2")` is `Int(2)`; `decimal("2.0")` is `{20, 1}`).
Structural equality distinguishes scales. A name never changes
validity: `val p = decimal("1.50")` folds to a declared-scale
constant, and the validator and decoder share
`declared_decimal_well_formed` (budgets + the written-scale fold).
The wire seals a decimal at the scale it CARRIES — three origins,
one rule: typed = declared, document = minimal (at the reader),
constructor = declared (identity.md tags 6/10). Statically
`decimal("…")` is `Dyn` in this build; inferring a true
`decimal(p, s)` static type is the long-term path and awaits the
type-system slice.

Consequence — **package format v6**: `decimal` enters the plan's
call alphabet (fn id 4); the categorical rule bumps the version
(v5 joins the refused drafts, future versions refuse by number).
No identity moves: the semantic hash covers the HIR canonical
form, not the package (v4 precedent) — goldens re-blessed for the
version byte alone, and the embedded spike re-proves the same hash
on target.

**Superseded drafts, by name** (their text lives in Git history):
the `json <value>` clause of D-2026-08-12-1 (family 4 — documents
only now) and its "`json null` runs" example (the null VALUE still
executes — `json [null]`, or a decoded `null` line; the bare
emission stays deferred with its static type); the
`json <json-value>` scalar enumeration of D-2026-08-10-6 (its
braced-map/array clauses stand); the grammar draft claiming
`DECL_KW` unmaskable; this decision's own round-1 "scalar refusal
with a hint" and round-1/2 "any other follower is CEL" clauses
(both fell when `json` became fully reserved); the round-3
paren-tracking in the json recovery (split into the two walks);
and the round-2/3 probe/stitch surface
(`significant_on_line`-as-parser-API, `cel_expr_from`,
`skip_balanced_raw*`, `advance_to`, `source_from`, the parser-side
lookahead) — all replaced by the lexeme-stream frontier above.

## D-2026-08-12-1 — Node attributes: one syntax, one contract metamodel

A configurable property of a Node is an **Attribute**, and every
attribute is written `name = value` — no other form. The value
belongs to exactly one of FOUR families:

1. **Bare word** — a closed choice owned by the contract
   (`mode = job`, `codec = jsonl`). `mode = "job"` is NOT
   equivalent: it supplies CEL text where the contract expects a
   choice.
2. **Expression** — CEL, constant in boot-phase positions. A tag
   with parenthesized positional arguments and NO body lives here
   (`duration("5s")`, `bounded(64)`, `asset("logo.png", ct?)` —
   `asset` merely resolves at build time).
3. **Braced literal** — struct (`=` entries) or map (`:` entries),
   `{}` / `{:}` for the empty forms, replaced ATOMICALLY (no
   implicit patch: `tls.verify = true` is not a patch of `tls`).
4. **Foreign literal** — `Tag [(binding args)] body`, the body
   owned by the tag's reader: guests (`jq`, `sql`, `bash`, …, with
   optional binding `jq(request) { … }`), the raw literals `text`
   and `base64(ct?)`, the data formats (`avro`, `protobuf`,
   `parquet`, `msgpack`, `csv` — reserved), and `json <value>`,
   whose grammar is self-delimiting and therefore needs no braces
   (`json 1`, `json null` — the M1.2 `[type, root]` identity is
   untouched).

**Killed, without replacement:** (a) configuration sub-blocks —
`env { … }`, `init { … }`, `tls { … }`, `jwt { … }`, and the named
form `auth oauth2_client_credentials { … }` are no longer forms;
the canonical spelling is a braced-literal attribute
(`env = { "K": v }`). This removes the property/namespace
ambiguity, the implicit `env.X` flattening, and group activation
rules. (b) **Tagged initializers** — `on_crash { … }`,
`measure { … }`, `exponential { … }`, `linear`, `constant`,
`fibonacci`, `oauth2_client_credentials` leave the tag vocabulary
entirely: a plain struct plus the attribute's name suffices, and
where a DISCRIMINANT existed it becomes a closed-choice FIELD of
the struct (`Loop.backoff = { curve = exponential, … }`,
`auth = { scheme = oauth2_client_credentials, … }`). Both dead
forms refuse with a migration hint, never a generic error. Dotted
attribute names remain ONLY for owner qualification
(`Loop.backoff`, `Saga.timeout`).

**The contract metamodel** (module `rastok::contract`, compiler
layer — never serialized, nothing enters `rastok-core::node`): a
node declaration resolves a contract, the contract may select a
PROFILE through one attribute (`Process`: `mode`, default `job`),
and every attribute carries a `Presence`
(`Required | Optional | Defaulted(DefaultValue)`) and a
`ValueDomain` (`Choice | Text | TextList | Duration | ByteSize`
implemented — `ByteSize` added in the review round: a declared size
is `<digits><unit>` with unit ∈ `B | KiB | MiB | GiB`, ASCII
whitespace tolerated around the size and between digits and unit,
an explicit unit required, at most 4 GiB
(target-independent: the parse is `u64` on every machine, and the
adapter compares lengths in `u64` — what validates is what
executes); `CelPath`, `Struct`, `Map` declared and admitted only
behind a DEFERRED profile). `Defaulted` absence is sugar: the checker
injects the contract's default — a `Choice` word directly, a CEL
default through the SAME chain as author expressions (compile →
const-eval → domain judgement) — into the HIR BEFORE
canonicalization, so omission and the exact spelling of the
default are ONE identity — same canonical identity (hash) and same
plan; the raw HIR keeps source order with injected defaults
appended, and canonicalization erases the difference. This does not
merge distinct constant expressions (`duration("60s")` and
`duration("1m")` keep two trees). `Optional` absence is a real
state that survives into the plan. The plan carries EFFECTIVE
attributes only (existing `ConnectorSpec.attributes` pairs — v5
untouched); adapters CONSUME resolved values and never
reintroduce a default (an incomplete plan is a violated
invariant, not something to complete silently).
`AttrDomain::Forbidden` is REMOVED without replacement: a field of
another profile is simply absent from the current one, and the
diagnostic names it (« `restart` belongs to
`Process(mode = service)` … »); a fixed value is a one-word
`Choice` (« `on_timeout` is fixed to `kill` … »). Profile
availability is the BUILD's fact (`ProfileSupport::Deferred`),
distinct from the contract knowing the profile; `mode = service`
defers before any service-attribute validation. The `Process`
table: `command` Required, `args` Defaulted(`[]`) (absence ≡ `[]`;
`null` refuses — a domain admits `null` only if explicitly
nullable), `codec` Required (REAFFIRMED: framing changes bytes,
digest, reply type and error surface — never chosen silently;
D-2026-08-11-1 holds), `effect`/`behavior` Required, `timeout`
Optional, and on the job profile `max_output`
Defaulted(`"8MiB"`), `on_timeout` Choice(`kill`) Defaulted.
*Amended 2026-08-12 (Process capture):* `max_output` is the
AGGREGATE capture budget across stdout and stderr, judged in `u64`
before any byte is stored — neither stream ever loses bytes
silently, and a stderr flood trips the same verdict. The
`OutputOverflow` payload is `{ limit, reason }` with
`reason ∈ budget | capacity` (the host's capture allocation refused
via `try_reserve`): NO `stream` field and NO exact `size` — the two
pump threads have no total order, so naming the crossing stream or
a drained byte-count would journal a thread race, unstable across
runs of a deterministic program. A pipe READ error is not an EOF:
it travels as its own fact and refuses as `CaptureFailed` — its own
kind in the Process error set (an I/O loss is not invalid content),
with a CLOSED, EMPTY payload: the OS detail and the failing stream
would race between two pumps. `Interrupted` retries; truncated
bytes are never presented as the whole. When several terminal facts
are observed, the verdict reduces by FULL fixed priority —
`Budget` over `Capacity` over `CaptureFailed`, whatever order the
pumps delivered them in: `Budget` is deterministic in the observed
totals, so a later budget crossing UPGRADES an earlier capacity
refusal, and the environmental facts never mask it. The kill
happens at the first BOUNDED drain containing any terminal fact
(one drain takes at most a channel's worth, so control always
returns; what a later drain would have shown is unknowable by
construction). Adding the kind to the
travelling error set is a DECLARED identity move — every Process
graph moved once (echo/spike `c75b840d…`). `ExecFailed` has ONE shape — `{ exit_code, stderr }`,
a wait/OS bookkeeping fault reporting `exit_code = -1` with the
fault in `stderr`. The adapter's emissions are an enum
(`ProcessFailure`) whose payload shapes seal with their kinds; the
binding proves adapter kinds ∪ machine kinds (`TimeoutExpired`)
⊆ the travelling `error_set`, and a CALLABLE connector of another
contract refuses at binding (`UnservedConnector`) — the global
`BoundExecutionPlan` router being the second adapter's named
design.

**Rejected:** structured constructors with named parenthesized
arguments (`on_crash(backoff = …)`) — a struct field does the
same job inside the one braced form; a `DefaultValue` algebra
beyond `Choice`/`CelExpression` (text, lists, durations are
already CEL); serializing any of the metamodel; `ReadOnly` or
`NotApplicable` in the value domain.

## D-2026-08-11-1 — The jsonl wire is canonical, both ways

The first structured codec (M1 item 2): `codec = jsonl` on
`Process`, job mode — one JSON value per line, the JSON model
exactly. The OUTBOUND serialization is CANONICAL because its bytes
are `request_digest` material: object keys sorted by their UTF-8
bytes, structs and maps alike — two values one identity owns must
not produce two digests. This is the WIRE's canonical order, not
the seal's: the envelope seal sorts map entries by their tagged
CBOR bytes (length-prefixed, so `"b"` precedes `"aa"` there) while
the wire writes `"aa"` before `"b"`; both are deterministic, and
each authority owns its own bytes. `int` writes as the plain
integer, `decimal` in plain decimal notation at its sealed scale
(the exact number survives the wire round-trip), `double` in
shortest round-trip form, one trailing newline. Refusals are typed
`DecodeError`, never silent: `bytes` and encoded payloads have no
JSON form (`lines` carries opaque text/bytes; a `bytes` codec stays
reserved), and a non-finite double is not JSON. INBOUND decodes
eagerly through the house reader — exact numbers, duplicate keys
refused, the offending line carried in the error payload, empty
stdout = the empty array, the trailing newline's empty tail is
framing. The reply of a job is `data = array<variant>` (the sheet's
rule, possibly length 1). Service mode stays deferred with its
stream machinery. **Rejected:** insertion-order object keys (two
by-name-equal values would carry two digests); base64-ing `bytes`
into strings (a silent representation change the reader could not
undo); skipping unparseable lines (a lost line is a lost fact).

*Amended 2026-08-11 (review round):* five sharpenings, each with
its regression. (1) **`null` executes** — the envelope domain
gained its null value (identity tag 9, shared with the CEL twin),
so `json null` runs and a `null` line decodes; "the JSON model
exactly" is now literally true and the refused forms remain
exactly bytes, encoded payloads, and non-finite doubles.
(2) **The reply's type is a real pin** — the contract rule
`Process(job, jsonl).out = array<variant>` is spelled once
(`RequestCodec::reply_type`) and consulted by the checker (a mock
answering a bare record refuses at check) and by the machine
(every settled outcome is judged against it): mock and world, one
shape. `lines` stays untyped by NAME — the frozen referee corpus
mocks structured replies over it; its honest typing lands with the
connector sheets. (3) **The codec's domain is validity** — the plan
validator refuses any `codec` outside `lines | jsonl`
(`UnknownCodec`), so no ValidatedExecutionPlan is ever silently
reinterpreted as `lines`. (4) **Framing is exact** — empty stdout
is the empty array but a LONE `\n` is one empty line and refuses
(the framing is judged before the terminator is stripped); a line's
padding is JSON's own whitespace alphabet, nothing wider — a
non-breaking space is trailing content. (5) **The decimal128 domain
is held at the doors** — core `Value::validate` refuses an
out-of-domain coefficient or scale before it travels, while
`datatype::decimal_in_domain` is the shared logical-domain authority;
the writer keeps a defensive mirror plus the reader's nesting budget:
a hostile scale is a typed refusal, never a runaway allocation.

*Amended 2026-08-11 (second round):* four more edges.
(1) **The effective pin is ONE derivation** —
`boundary::connector_out_pin` (Process-scoped) is the single
semantic authority; the checker's port-pin lookup (edge audit AND
mock-reply judge), the plan validator's edge audit, and the
machine's settle judge all consult it. A `shell.out -> sink.scalar`
edge from a jsonl connector to an `int` port now refuses at CHECK
(E-PORT-TYPE, `List(Dyn)` vs `Int`) instead of surfacing as a
runtime fault. (2) **`codec` is the Process sheet's name, not a
global one** — the validator's domain-and-presence rule fires only
for `spec.contract == "Process"` (the sheet REQUIRES the attribute,
so a Process plan without one — `MissingCodec` — never left the
compiler); a future sheet spelling its own `codec` vocabulary is
untouched. (3) **Doubles write TRUE shortest round-trip** — the
forced `.0` is gone: the inbound domain is dynamic, so an integral
double's line legitimately reads back as `int` (the static tag does
not survive the wire), and forcing a fraction only made the form
non-shortest against this decision's own words. (4) **The wire is
GOLDEN** — `examples/jsonl.rkg` pins the fixture's semantic hash,
its package, and a full Lab trace whose OUTBOUND record seals the
canonical line's `request_digest` (key 43), verified equal to the
digest re-derived from `prepare_jsonl`: a change to the canonical
form must re-bless the trace with its reason.

## D-2026-08-10-1 — Metadata has exactly three zones

`meta.sys` (runtime-owned, read-only), `meta.app` (application-owned,
read-write), `meta.ext` (external-boundary context, read-write). No
other root exists: no bare field (`meta.customer_id` is invalid — the
`meta.x ≡ meta.app.x` alias is REMOVED), no custom namespace at the
root, no connector name in a meta path, no lexical scoping of meta
references, no prefix-freedom machinery. `app` is auto-available and
its fields are INFERRED from the DEPLOYED GRAPH's statically visible
writes ALONE — the Lab is checked against that resolved schema and
extends nothing:
types unify across writers and a conflict refuses with both sites
named; a read of a name written nowhere refuses (the existing
CEL-grid rule); dotted names (`meta.app.audit.origin`) are plain
field names, not a subspace mechanism. There is NO declaration form
for `app`. `ext`'s vocabulary is declared ONLY by connector contract
sheets (a `Populates` row: fields and scalar types, unified across
connectors — incompatible declarations refuse naming the connectors
and types) plus the standard reserved field `content_type`
(D-2026-08-10-4); handlers may REWRITE declared ext fields, never
invent one — the application's own vocabulary belongs to `app`.
Leaves stay `scalar | array<scalar>`; composites live in `data`. The
physical form is FLAT — one dotted string key per field within its
zone (`"app.audit.origin"`, `"ext.content_type"`); the projections
NEST by splitting on dots, so CEL reads `meta.app.audit.origin` and
a guest block sees nested objects, while every seal sorts by the
flat key and the package's schema tables key by it — one storage
form, one sort order, nested views derived. `meta`, `data`, `sys`,
`app`, and `ext` are refused as node names. **Supersedes:** the root
alias and its write/read projection (round 5 of M0); root custom
namespaces (`meta.<custom>`); every intermediate proposal of
connector-path namespaces, scoped resolution, shadowing,
reach-through reads, and prefix-freedom (none implemented).
**Rejected:** a mandatory `meta app { … }` declaration (ceremony the
grid already covers: the read-of-nowhere-written rule and
cross-writer type unification catch the dangerous typos — the
write-only typo mints a dead field and is the accepted price);
per-connector namespaces (two connector types wanting one name,
unanswerable join merges, renames rippling through readers).
## D-2026-08-10-2 — Propagation is uniform; joins merge by value

A single-input emission copies `app` and `ext` from the input, mints
`sys` fresh, then applies the handler's explicit writes,
last-write-wins per field. A join has NO implicit parent: for every
field of the statically known universe (the inferred `app` schema —
every write in the unit —, sheet `ext` fields, standard fields), the
merge is CommonOrAbsent — absent
everywhere → absent; one distinct value → kept; conflicting values →
absent — using each metadata type's canonical equality, never a map
order or a scheduler order. The compiler and Registry-free validator derive a
static per-field presence verdict from the canonical schemas, topology, and
explicit writes; the runtime executes the corresponding merge. The earlier
proposed travelling `JoinMetaPlan` is superseded by the R6 checkpoint of
D-2026-08-18-1: duplicating already-transported schemas would create a second
claim to reconcile. `content_type` is NOT part of the fusion — it
derives from the payload decision (D-2026-08-10-4). PRESENCE IS
STATIC: past a join, a field whose merge can yield absent (a
conflict is topologically possible, or no input is guaranteed to
carry it) is OPTIONAL — an optional read (`?field`) stays permitted;
a required read, or a required output contract, refuses at check
until an explicit write has chosen the value. An unused conflict is
silent — the refusal sits at the READ or the contract, never at the
merge itself. The runtime's missing-field fault remains defense in depth for
invalid plans, never the primary check and never an implicit `err` emission. Reads never
propagate — a `val`, a guard, or a dead computation changes no
output metadata; only the merge and explicit writes do, and explicit
writes win. In a join handler the read forms are `a.meta.*` /
`a.data`; bare `data` refuses (two payloads, no referent), and the
laziness witness is KEYED per input — `Payload(a)` / `Payload(b)`,
one slot each. `meta.sys` never merges. **Rejected:** trigger-meta
inheritance (an implicit parent by another name); runtime map
iteration; runtime-only presence (a required field silently
vanishing at a join is exactly the class of bug a checker exists
for).
## D-2026-08-10-3 — No history operator; the journal is not a handler input

There is no `last`, no causal cursor, no per-plan history dependency,
and no `HistoryUnavailable` in the language. If the application will
need a value after its replacement, it copies it into `meta.app`
while it is current — the same idiom the HttpServer `correlate` rule
already mandates ("the author carries the token"). The separation is:
`meta.ext` = the current external context; `meta.app` = what the
application decides to keep; the journal = past versions, for audit,
observation, replay, recovery, and diagnostics — never consulted
during semantic evaluation. The VM needs no journal access to
evaluate an expression. There is no HISTORICAL `last(...)` operator
of any spelling — the existing `last N` join count bound
(grammar.md) is untouched and keeps the keyword. **Rejected:**
`last(connector, path)` with causal-nearest semantics and a carried
cursor (deterministic, but a hidden state machine the reader cannot
see; the explicit copy is visible, cheap, and already the house
idiom).

## D-2026-08-10-4 — `content_type` is a standard `ext` field, coupled to the payload

The canonical name is `content_type` (never `mime_type` — the value
is the FULL Content-Type, parameters included, so the old name
contradicted its own definition; the NiFi-lineage rename note dies).
Type `text?`; read as `meta.ext.content_type`. There is no
`meta.sys.content_type` and no `Data::Encoded.content_type`; an
`app` field named `content_type` carries NO standard semantics — it
is ordinary application vocabulary like any other name; the `meta.sys` spine drops to FIVE
fields (id, parent_id, origin_id, role, graph_version) and the seal
registry's mime row moves to the ordinary `ext` seal — the field
stays causal. `WriteTarget::MimeType` and `Draft.mime_type` die.
Emission construction is atomic, in this order: (1) fuse or copy
`app`/`ext` per D-2026-08-10-2, `content_type` EXCLUDED; (2) decide
the payload — FRESH data, or a KEPT witness `Payload(x)` of one
input; (3) derive `content_type` from that decision: fresh → absent,
`Payload(x)` → input x's value — never a fusion with the other
inputs' (the payload-coupling invariant: a content type describes
exactly the bytes it was born with; the old ad-hoc mime-nulling rule
REFORMULATED, not removed); (4) apply explicit writes, which win;
(5) mint one envelope, `sys` fresh. No intermediate state is
observable or journaled. The witness rule covers every spelling of
the carry — `out = .`, `out = data`, `out = a.data` in a join,
`val copy = a.data; out = copy`, a ternary whose taken arm is a
witness, and the whole-data write `data = a.data` — the decision
follows the VALUE (the keyed witness), never the syntax. Inbound,
the connector writes `ext.content_type` and selects/validates the
INTERNAL encoding at the frontier; the core decodes with
`Data::Encoded.encoding` alone, never consulting Meta; v1 decodes
HTTP JSON eagerly at the connector — `Raw` is the only deferred
regime until the core carries a decoder, and a JSON body is never
parked under `Raw` where a structural access would materialize it as
raw text. Outbound, a serializing codec dictates the produced
content type and writes it; a byte pass-through uses the current
value if present and compatible, else the connector's configuration,
else `application/octet-stream`; a codec/content-type contradiction
is a TYPED error, never a silently false header. The four planes
stay distinct and get their table in envelope.md: logical type /
internal encoding / connector codec / external content type.
**Rejected:** fusing `content_type` like an ordinary ext field (two
equal input types on a FRESH payload would survive the merge and
describe bytes that no longer exist — the one case the invariant
exists to kill); `meta.app` placement (uniform inheritance makes a
stale type lie by default); `Data::Encoded` placement (right
coupling, wrong owner: re-labeling needed a dedicated function, and
connectors could not write it as ordinary context).
## D-2026-08-10-5 — Build-time bytes: `asset` and `base64`, content-typed at whole-data only

`asset("path", content_type?)` and `base64(content_type?) { … }`
construct, atomically, a fresh byte payload plus an explicit
`ext.content_type` — desugared to a whole-data byte source and an
ordinary `Ext` write, no dedicated plan op. The content-typed forms
are legal ONLY in whole-data position (`out = asset(…)`,
`data = asset(…)`): a nested field has no `meta.ext` of its own, so
`{ document = asset("x", "ct") }` refuses with a dedicated
diagnostic. The bare forms embed anywhere as plain bytes and, in
whole-data position, leave `content_type` ABSENT — never inheriting
the replaced payload's. `asset` invariants: build-time resolution
only; path relative to the App root; escapes and missing files
refuse at build; raw content, no normalization; the semantic hash
seals the CONTENT DIGEST, never the path (spelling); the bytes live
in a content-addressed package section outside the decoded body.
**Amended 2026-08-10, at implementation:** the v1 physical form is
body key 7 — raw asset bytes in strict DIGEST order (the checker
sorts and remaps indices at seal, so reordering source declarations
moves nothing), digests DERIVED at decode (and thereby verified),
with order, uniqueness, digest truth, and both budgets re-checked by
the validator. The out-of-body section deferred WITH the embedded
loader that needs it: v1 embedded targets refuse asset-bearing
plans, so no decoder pays the in-body cost today, and moving the
region is a format bump when the loader exists. Content addressing
(the digest as the sole identity) is unchanged.
Budgets: 8 MiB per asset and 32 MiB total — FIXED caps in v1, not a
knob (an App must not raise its own resource barrier from its own
source; a future version may lift them on evidence, which breaks
nobody, where a later tightening would); breach refuses at build.
The v1 EMBEDDED profile does not implement the asset section — a
loader-scope limit, so an asset-bearing plan refuses at boot with a
typed error naming the missing capability (streaming digest
verification is perfectly possible in 256 KiB; what is absent is
the loader, deliberately, until a real embedded case pays for it).
The VM never touches a filesystem; runtime files and URLs are
Connectors' business. **Rejected:** no caps (a cap added later
breaks existing apps; one relaxed later breaks nobody — the ratchet
only turns one way); App-declared overrides (the barrier would
guard nothing); URLs; path-sealed identity.
## D-2026-08-10-6 — Literal reform: punctuation decides, one spelling each

`{ a = 1 }` is a struct; `{ "a": 1 }` is a map (text keys in this
milestone); `{}` is the empty struct; `{:}` is the empty map; mixing
`=` and `:` entries refuses. Values NEST recursively — maps, json,
the empty struct, and the bare byte/text forms are ordinary nested
values — with ONE inexpressible form in v1: a non-empty struct
inside a map (no serializable struct-literal expression exists);
it defers by name, and `json` covers dynamic nesting at the price
of being `variant`, never a typed struct. `map { … }` dies — superseding exactly
that clause of D-2026-08-09-1 (its braced-classification core
stands). The CEL MapLit aligns on the same punctuation: one map
dialect per source file. `text { … }` is the verbatim text form:
the first newline is excluded, the closing brace's column sets the
margin stripped from every line, the trailing newline before the
brace is excluded, UTF-8 mandatory, no interpolation and no escapes,
braces must balance (the raw-block lexer's rule — content with
unbalanced braces uses a CEL string or `base64`; a named
limitation). `json <json-value>` replaces `json { { … } }`: object,
array, string, number, true, false, null — always `variant`;
duplicate keys refuse. `base64 { … }` is the SOLE inline binary:
RFC 4648 standard alphabet, mandatory padding, ASCII whitespace
ignored, URL-safe refused. `hex`, `bytes base64`, `bytes hex`, and
`<type> file` die — `asset` (D-2026-08-10-5) replaces `file`.
**Rejected:** type-directed braces (the syntax decides, never the
expected type — the reform's founding rule); keeping `hex` (two
spellings, one semantic).

## D-2026-08-10-7 — JSON numbers are exact: decimal-anchored, integers at honest width

A JSON number parses to an EXACT value, and the identity registry's
existing rows carry it — no new tag. Canonicalization of a dynamic
(`variant`) number: normalize the decimal (strip trailing zeros); a
scale-0 value in the signed 64-bit range enters the INTEGER domain
and seals at its honest width — tag 2 (`int`) within the 32-bit
range, tag 3 (`long`) beyond — the same rule `int_tagged` already
applies everywhere; `-0` normalizes to integer zero; everything else
becomes a DYNAMIC decimal on tag 10, whose layout is already pinned
(`[unscaled bytes, scale]`) with one context difference: a TYPED
`decimal(p, s)` seals at its DECLARED scale (the registry's rule,
unchanged — `decimal(p,2)` value 1 is `[100, 2]`), while a dynamic
decimal seals at its MINIMAL scale (trailing zeros stripped, so it
never overlaps the integer domain). The budget is Arrow's
`decimal128` anchor: 38 significant digits, decimal128 exponent
range; beyond it, REFUSAL — a build diagnostic in literals, a typed
error at decode. So `1`, `1.0`, and `1e0` are one identity (int 1);
`19.99` is exact; doubles arise only from explicit narrowing
(`as double`), never from parsing — "numbers are lossless until
narrowed" is honored by refusing loss. **Supersedes, by name, three
clauses of identity.md's tag-6 (`number`) canonicalization:** the
`-0.0 → double(5), never fold to long 0` rule (a dynamic `-0` is
now integer zero; `-0.0` as a TYPED double, tags 4/5, keeps its
sign — that rule is untouched); the `integral in i64 → long(3)`
rule (honest width now — tag 2 within 32 bits); and the
`non-integral binary64-exact → double(5), beyond → TYPE ERROR
unless declared decimal` rule (non-integral now goes to the dynamic
decimal, refusal only past the 38-digit budget). None of these
clauses had an implementation, so no golden moves. Implementation:
a HOUSE JSON reader in rastok-core, justified by a survey, not by
doctrine alone — `serde_json` (+`arbitrary_precision`) fails on
rustc-style spans into the `.rkg` (it reports positions in its own
stream) and silently keeps the last duplicate key; `serde-json-core`
(no_std) has neither arbitrary precision nor spans; `tinyjson` /
`json` are std-bound without span access; `sonic-rs` is
unsafe-heavy for a kernel — and none can live in a zero-dependency
no_std kernel whose acceptance must not drift under a version bump
(D-2026-08-01-1, D-2026-08-03-1). `serde_json` joins as a
DEV-dependency differential oracle, scoped to what it can actually
verify: the COMMON domain — inputs both parsers accept (valid JSON,
≤ 38 significant digits, no duplicate keys) must yield identical
trees; Rastok's refusals (duplicate keys, over-budget numbers) are
asserted directly, since the oracle cannot express them.
**Rejected:** f64 by default (silent corruption of `19.99` and of
integers beyond 2^53); unbounded bignum (a numeric tower this
milestone does not need); `serde_json` in production (the survey
above).
## D-2026-08-10-8 — Named types are structural sugar

`type Name = <type_ref>`: a unit-level namespace, duplicates
refused, and the semantic hash seals the STRUCTURAL EXPANSION —
renaming an alias without changing its structure is not an identity
move (the sugar-insensitivity rule, D-2026-08-04-2, applied to
types). Recursion refuses in v1: a recursive type is inexpressible
in the plan's resolved-type tables and has no depth under
`MAX_NESTING`; a μ-form waits for a nominal type table in the
package, if evidence ever demands one. Budgets, exact: nesting
depth ≤ `MAX_NESTING` (128, the existing format budget); an
EXPANDED type tree ≤ 4 096 nodes total; ≤ 256 fields per struct
and ≤ 256 symbols per enum; ≤ 64 KiB of canonical type bytes per
plan — each refusing at check with the measured value in the
diagnostic, and re-verified by the package validator.
**Amended 2026-08-10, at implementation:** the delivered slice
enforces STRUCTURAL depth, total nodes, per-struct/per-enum counts,
and an ALIAS-FOLLOW budget of 256 links at collection — a name
carries no structural weight, but following one is work, so a
50 000-link chain refuses typed instead of exhausting the compiler;
pure alias chains then COLLAPSE once, after validation, so every
use site resolves in one hop and what validation accepted,
resolution serves (the boundary chain keeps its variable and its
hash); the 64 KiB canonical-byte budget and its
package re-verification land WITH composite pins — until a
composite type can reach a plan, there are no canonical type bytes
to measure. Composite ALIASES resolve structurally but still meet
the pre-reform scalar port subset (`E-CHECK-DEFERRED`), a scope
older than this decision.
The type tree itself is unchanged — `scalar`, `struct`, `map`,
`array`, `enum`, `union`, `variant`; `table` stays the relational
view of `array<struct>`. `schema` is not introduced (it would only
carry meaning under nominal identity, which is rejected).
**Rejected:** nominal identity (renames would move hashes);
recursion via named references (reopens the plan format and the
depth budgets for no current need).

## D-2026-08-10-9 — One format bump: v5

The whole reform ships under ONE new package format. v5 carries: the
`App`/`Ext` write targets (`CustomMeta` and `MimeType` die); the
resolved `app`/`ext` schemas (they TRAVEL — the validator answers
without a Registry, the round-2 doctrine); the digest-ordered asset
table in body key 7 (D-2026-08-10-5 as amended); expanded named
types; the structural budgets; the `decimal` canonical tag.
**Amended 2026-08-10, at freeze (twice — the first freeze was
lifted the same day, pre-publication, to settle the implicit
`content_type` and the recursive value forms):** v5 is FROZEN as
landed — `ext_schema` carries sheet fields only (the standard field
is implicit), map values compose through the MapLit expression (a
non-empty struct inside a map is inexpressible in v1, deferred by
name), and the `JoinMetaPlan` ops and any out-of-body asset region
take LATER versions; this reader's re-encoding check refuses
unknown keys, so nothing rides v5 silently. It carries NOTHING for history lookups
(D-2026-08-10-3). The v5 skeleton lands with the first serialized
change and exactly one final number is published; v4 joins v1–v3 as
unpublished drafts refused by number — this entry SUPERSEDES
D-2026-08-09-2 as the current-format authority (its rule stands,
its number falls). **Rejected:** serial bumps
(intermediate states that are neither v4 nor v5); waiting until the
end to touch the format (every prior step would produce
unrepresentable plans).

## D-2026-08-09-2 — The empty-struct literal moves the package to v4

The reform's nested empty record (`parent = {}`) compiles to the one
struct constant a plan may carry: the EMPTY struct literal, tag 17.
Reading it back is an extension of the values a v3 decoder accepted,
and the format note's own rule is categorical — anything the tables
cannot express is a format-version bump, not a creative reuse. So
the package moves to v4, v3 joins v1 and v2 as unpublished drafts
refused by number, the goldens were re-blessed deliberately, and the
spike boots the v4 package to the SAME semantic hash — identity
lives in the canonical HIR, never in the transport. **Rejected:**
declaring tag 17 retroactively part of v3 (it contradicts the
written rule, and a rule bent once pre-public will be bent again
under pressure); accepting both v3 and v4 in one reader (two
accepted dialects is a silent migration surface — the single-version
reader stays the sabotage matrix's simplest honest shape).

## D-2026-08-09-1 — One braced form, classified once by its roots

A maintainer language decision, taken during the third multiport
review round, before anything public exists. The value grammar keeps
ONE braced form: `{ … }` is the record literal in every value
position, and in envelope positions (handler emissions, fixture
injections, mock replies) the checker classifies its top-level roots
exactly once — no reserved root builds a FRESH data record (a
`struct`, canonical tag 17), keeping the input's heritable metadata;
only reserved roots (`data`, `meta`, `mime_type` — a closed,
normative list) select the envelope form, where a whole-`data`
assignment makes the build fresh and its absence makes it a patch; a
mix refuses as `E-BRACE-MIXED`, never a silent reinterpretation.
Business fields that happen to be named like a reserved root take
the explicit escape `data = { … }`. `{}` is the empty record; the
exact re-emission of the current envelope is spelled `.`
(`out = .`); `map { … }` stays the explicit spelling of a map
(canonical tag 18). The `envelope` keyword forms (`out = envelope`,
`envelope { … }`) and the `struct { … }` value literal are REMOVED —
not aliased, not deprecated — while `struct` remains in type
positions. Migration invariant, held by the gauge fixture and the
goldens: a program converted spelling-for-spelling produces the same
HIR, the same validated plan, the same semantic hash, and the same
golden bytes — the reform touches the surface, never the identity.
**Rejected:** keeping `envelope { … }` beside the classified braced
form (two spellings for one build — the sugar/expansion split
produced the round-3 Map/Struct divergence); classifying by VALUE
shape at runtime (string decisions in the VM are exactly what
D-2026-08-04-2 removed); an open reserved-root list (any future
addition would silently reclassify existing records).

## D-2026-08-08-3 — A pinned port type is a public contract

> **Partly superseded by D-2026-08-14-1 and M1.3 B2.** An omitted annotation
> is now a compiler-only inference variable that must resolve before execution;
> `variant` is an explicit dynamic type, not symmetric inference territory.
> Precise → variant is assignable; variant → precise requires narrowing. The
> annotation remains identity material in v6, while the derived effective
> table is recomputed by validation until v7 transports it. D-2026-08-14-1's
> Operator-fault closure additionally supersedes the historical synthesized
> `err` recovery described below: conformance gates remain, but an engine fault
> commits nothing and reaches supervision.

Second multiport lift. A port annotation (`input evt : text`)
resolves through the same type subset as state cells (its deferrals
refuse identically) and becomes part of the port's declaration
everywhere the declaration lives: the handler's `data` takes the
pinned type; an emission, an injection, or an edge whose PRECISE
type disagrees with a pinned end refuses as `E-PORT-TYPE` (`dyn` on
either side is inference territory and passes), with builders held
to the ORDERED write fold the runtime executes — the checker and
the machine share one construction model, and the machine holds
CONSTRUCTED content to the pin at emission-build time with the
state-cell judge, so non-constant arithmetic cannot carry an
out-of-range int across an `int` port; the package carries the type
as `port_decl`'s optional third element — absent when unannotated,
so an unannotated plan's BODY bytes are unchanged, while the PACKAGE
moved to format v3 with the widening (the version is checksum
material); and the annotation is IDENTITY (identity.md: types are
sensitive), hashed with the var-init presence idiom. Unannotated
graphs with port tables gained one absence tag per port in the hash
— an identity move PINNED by the gauge fixture (exact hash, v3
package bytes, golden trace with INBOUND key 24). **Rejected:**
hashing the RESOLVED type (inference does not exist yet;
annotations-as-written is the honest contract), and SCATTERED
per-edge runtime checks — the adopted shape is TWO named
levels sharing one judge: a RECOVERABLE check at construction — an
operator's or partner's value breach converts into a typed error on
`err` — and the FINAL, non-recursive gate every commit candidate
passes (`validate_and_reserve_emissions`, mirrored at ingress by
`inject`), inspecting EVERY pin of the fan-out, synthesized error
payloads included; an error that violates its own `err` wiring
surfaces as a machine fault and nothing commits. A `variant` pin is
an explicit "anything" and never conflicts — checker, plan
validator, and runtime judge share that rule. Encoded external
bytes stay the codecs' door.

## D-2026-08-08-2 — The inbound record carries its ingress port

First M1 lift (multiport). An envelope may now enter through a
DECLARED boundary input, so the port becomes a causal fact — replay
must re-route from the same input — and it joins the INBOUND record
as key 24 (`ingress_port`, text), extend-only per the table
discipline: **omitted for the default `in`** (absent = omitted), so
every pre-multiport trace is byte-identical and no golden re-bless
is needed. The machine verifies the submission's port against the
plan (implicit `in` or a declared boundary input) and refuses
anything else typed (`UnknownIngress`); the checker resolves and
direction-checks the injection target, so a checked scenario never
reaches that refusal. **Rejected:** keying the record on the edge
list instead of the port (routing is the plan's job, the record
states the fact), and a required key 24 (would move every existing
trace for a default nobody wrote).

## D-2026-08-08-1 — A default port may be restated once

"Redefinable" in the grammar is bounded like every other
declaration: a default port (`in` / `out` / `err`) may be explicitly
restated at most once, in its implicit direction, and the
restatement never enters the serialized table — the trio stays
implicit everywhere downstream. A second explicit declaration, or a
declaration against the implicit direction, is `E-PORT-CONFLICT` —
the same rule an operator's `declared_names` applies. **Rejected:**
silently idempotent repetition (observable behavior with no stated
rule) and a blanket refusal of any restatement (contradicted the
grammar's own "redefinable", caught by review counter-example
`input in`).

## D-2026-08-07-3 — Package v2: framed checksum, total validation, canonicity by re-encoding

Format version 2 of the plan package. The checksum covers the whole
association — `blake3(domain ‖ format_version ‖ semantic_hash ‖
body)` with a fixed domain string and fixed-width fields — so a
tampered header refuses, not only a tampered body. After the checksum,
the decoder validates the plan structurally (slots in bounds, ports
and directions coherent, handler ownership, unique paths and vars,
var references resolvable) and enforces canonicity by re-encoding
what it decoded and requiring byte equality, which refuses non-minimal
integers, reordered maps, and every other non-canonical variant in one
total check. **Rejected:** checksum over the body alone (leaves the
declared identity unprotected — refuted by review counter-example);
per-rule canonicity checks in the CBOR reader (an open-ended list
where re-encoding is total); trusting the decoder's shape checks as
validation (a well-shaped plan can still name Slot 999).

## D-2026-08-07-2 — The expression uint domain gets registry tag 23

`uint` is a first-class expression value domain and needs its own identity;
reusing tag 3 (`long`) made distinct values byte-identical
(E-2026-08-07-1). The registry being extend-only, `uint` is appended
as tag 23; tag 3 is long only, and the package decoder maps 2 and 3
to the signed domain, 23 to the unsigned one. **Rejected:** refusing
`uint` at the encoding boundary (the value domain is real; refusing
it merely defers the collision to the first user); a package-local
tag diverging from the registry (one registry, everywhere).

## D-2026-08-07-4 — The codec lives under `plan`; validity is a type

Second review round. `plan_package` as a public module promoted the
artifact's name into an abstraction with no state or behavior of its
own; the codec is a capability of the plan — `rastok::plan::encode`
/ `rastok::plan::decode` (the pair keeps the repository's
encode/decode vocabulary). Structural validity becomes
`ValidatedExecutionPlan`: private field, `TryFrom<ExecutionPlan>`,
no unverified constructor, read-only access (`Deref`, no
`DerefMut`), owned extraction only through the explicit
`into_inner`, which drops the claim. The compiler's lowering earns
the type once; `Vm::boot` and `plan::encode` accept only it, so
"serialize accepts ⇒ deserialize succeeds" holds by construction.
The magic `RKPB` is pinned as "Rastok Plan Binary" — the expansion
names the byte form the magic labels, not a public type; D-2026-08-07-1
rejected `PlanBinary` only as an API name. **Rejected:** a
public `plan_package` module (name-as-abstraction), `serialize` /
`deserialize` naming (two vocabularies for one act), validating in
both the deserializer and the type (one validation, at the type).

## D-2026-08-07-1 — The transport artifact is a plan package, not a blob

> **Superseded in part by D-2026-08-07-4** — the module placement
> (`rastok_core::plan_package`) did not survive the next review
> round: the codec lives under `plan`. The artifact vocabulary
> ("plan package", `.rkp`, encode/decode/checksum, `seal` reserved)
> stands.

The compiled artifact is named **plan package** (extension `.rkp`):
a structured, versioned, validated product of the compiler — nothing
about it is opaque or generic. The API vocabulary is `encode` /
`decode` / `checksum`: an unkeyed BLAKE3 is an integrity checksum,
and `seal` is reserved for a future property that actually binds
trust (a signature). **Rejected:** `blob` (dishonest about
structure), `PlanBinary` *as a public type or module name* (suggests
machine code where a validated structure lives — the magic's
expansion in D-2026-08-07-4 names the BYTE FORM, which is exactly
what a magic labels; the two decisions are about different things),
keeping `seal`/`open` (overclaims a trust property the format does
not have).

## D-2026-08-06-1 — Specs are maintained in place; tags are the editions

Edition-in-place with git as the revision record, tagged editions
(`v0.0-spec-freeze`, a consolidated `v0.1` at public opening) as the
citable versions. **Rejected:** immutable documents with a
supersession chain (IETF-style `Updates:`) — right for a corpus with
independent external implementers, premature and fragmenting for a
co-evolving single implementation; per-file hand-maintained revision
blocks — redundant with git.

## D-2026-08-06-2 — The call identity derives from the delivery reference

`delivery_ref = derive(0x20, canonical_edge, envelope_id)`;
`call_ref = derive(0x10, delivery_ref, request_digest)`
(envelope.md §2). One concept carries every per-delivery obligation:
budgets, queue lookups, call identity. **Rejected:** salting
`request_digest` with routing information — its definition is the
exact prepared bytes and must not move; keying by
`(connector, envelope)` — two ports of one connector would collide
(review 10 of M0).

## D-2026-08-03-1 — Canonical bytes are hand-rolled deterministic CBOR

A standard substrate (auditable without our code, schema-free
navigation for extend-only tables) with a hand-written encoder for
the closed subset, because the requirement is byte identity under our
profile, not conformance — the profile deviates from RFC 8949 §4.2
where identity demands it (fixed-width floats, canonical NaN).
**Rejected:** a fully bespoke byte format (re-specifies the plumbing
minus decades of review); encoding libraries (their legal encoding
choices can change under a version bump — each change a broken seal);
`postcard` for canonical forms (not canonical, implicit schema).
Rationale in full in `golden.rs`'s module documentation.

## D-2026-08-01-1 — The compiler front end is hand-written

Recursive-descent parser, one token of lookahead, house lexer, house
diagnostic rendering; zero parsing dependencies. Chosen for
rustc-style error output and a dependency-light core. **Rejected:**
logos + chumsky + ariadne (an earlier private-corpus decision,
superseded by the frozen grammar's own statement and by practice —
the ariadne fallback reserved during M0 was never needed).

## D-2026-08-04-1 — Only the scheduler mints a timeout

A partner answers or stays silent; budgets arm at delivery (queue
time counts) and expire in the scheduler, which cancels in-flight
work through the Partner seam. **Rejected:** adapter-side wall-clock
deadlines fabricating `TimeoutExpired` as a failure response — it
made the response/expiry race unreal and put a causal decision in
the world's hands (reviews 6–7 of M0).

## D-2026-08-04-2 — Envelope writes resolve at check, never at run time

Builder paths become typed `WriteTarget`s during checking; the
runtime executes a verdict. The as-written path stays hash material,
but the hash seals the RESOLVED target (sugar-spelling-insensitive).
**Rejected:** runtime string re-interpretation — it produced four
confirmed misroutes (review 5 of M0); hashing the surface spelling —
it broke identity.md's sugar insensitivity (review 6).

## D-2026-08-05-1 — The core splits before it grows

`rastok-core` (no_std-ready: vocabulary, envelope, CEL, plan,
journal, boundary, machine) separates from the std layer (compiler,
adapters, lab, golden, CLI) at M0.5, while the surface is small; the
CI checks the frontier on a thumb target; neutrality is proven by the
unchanged semantic hash. **Rejected:** splitting later (every month
of growth raises the cost); putting any connector in the core (even
`Process` — effects are the std layer's business).
