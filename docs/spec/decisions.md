# Design decisions

> One entry per settled decision: what was chosen, why, and what was
> rejected — so a rejected alternative is not re-litigated without new
> evidence. Companion to [`errata.md`](errata.md) (defects found);
> git history carries the full context of each entry.

## D-2026-09-14-2 — Resolve unit-global constants in the App and its Lab

Yoann requested that repeated fixture identifiers use ordinary top-level
`val`s, with more local declarations keeping their lexical scope. The parsed
unit declaration now executes as a pure plan constant. It is checked before
Graph/Lab consumers; initializers use earlier unit constants in selected-file
and source order. No forward references, mutable global store, environment
lookup or new Lab declaration form is introduced. All declared constants are
checked, including unused ones; duplicates, shadowing and runtime dependencies
refuse with existing source diagnostics.

Constants substitute through the existing expression algebra before production
HIR. Scalar-literal references preserve the identical literal tree; composites
and computed values retain their checked types and lazy narrowing/read failures.
Literal Contract choices are not captured by same-named constants. Annotated
unit constants remain a named refusal, including through a type alias: policy
must not vanish when a binding is erased. Graph/Operator annotated bindings
remain the supported path.

No package, Contract, journal or type tag changes. Unused and Lab-only constants
do not affect production identity. Owner-local values retain their existing
phase and identity rules. The agent gateway's session and thread identifiers
move to unit constants; its goldens are checked unchanged, not re-blessed.

Rejected: duplicating long IDs in each scenario, inventing fixture-only
constants, implicitly shadowing an outer binding, and serializing a global
constant table into every production plan.

## D-2026-09-10-1 — Route explicit instance closure without restoring Graph handlers

**Accepted design, executable package v18 amendment.** Yoann approved a command-shaped
entry to the existing instance lifecycle, including closing only a child
Process while its enclosing conversation remains alive, then approved execution
of the existing-target path with idempotence per instance for every Contract.
This is the named exception to the v17 opening freeze. The implementation and identity audit is
in [instance-stop-command.md](../architecture/instance-stop-command.md).

In a Graph's wiring, `instance.stop` requests closure of that exact Graph
instance. `child.instance.stop` requests closure of one direct child's exact
instance, not the parent, its siblings or every instance at that child path.
A child Graph closes its own subtree; a child Connector closes its owned
exchanges through its Contract. This is a lifecycle command, not a new port,
PhysicalEdge, Connector, Extension or Graph handler. Unkeyed singleton owners
use the same rule. Bare `stop` in an Operator remains local and unchanged.

The static owner target is resolved in the plan; the generation-bearing
`InstanceRef` must be fixed before the producing transaction commits, from
retained instance context. A path, business key or `origin_id` alone is not
that reference. There is no global key lookup, wildcard, implicit child
creation or retargeting to a successor. Ambiguous instance selection refuses.
A socket-close cause needs an exact retained conversation binding; matching
the text of a user key does not confer that binding.

The producer atomically retains a bounded closure command with its other
products. The lifecycle controller consumes it and commits the target's
ordinary `l:closing`; the existing invariant that a closing record belongs to
its own instance remains. Pending commands survive recovery without rerunning
their producer. Repeated or superseded requests cannot close twice or touch a
successor. Their disposition must remain reconstructible. The command starts
Closing without waiting for Loop-style application quiescence; bounded drain,
settlement, exchange exits, inner-first stop and generation fencing are the
existing protocol, not a new one.

This completes the explicit request surface left open when D-2026-09-05-1
removed Graph handlers. It does not restore D-2026-08-18-1's retired remote
`stop <partition>`, alter Loop r2, make a Process exit a Node stop, or decide
whether a gateway should end a conversation when a socket disconnects.
Closure ends a generation, not the right to open a later generation at the
same address, and it does not itself restart a Process.

Commands are interface contributions, even though they are not ports. The
informal term "pseudo-port" describes their destination-like spelling only;
neither data nor control ports are fabricated. Existing Loop commands retain
their own typing and protocol. `instance.stop` belongs to common Node
lifecycle, not to an added Extension.

The [v18 identity ledger](identity.md#13-plan-v18--exact-instance-closure)
allocates destination tag 4, command ID domain `0x23`, sparse causal key 57 and
diagnostic recording v2 before blessing. Blessed v17 is not reinterpreted.
Process service/RPC decisions remain separate. Programs not using the addition
must retain semantic hashes and causal traces, with package-version movement
accounted for separately.

Selection walks retained exact parent-generation links: source self when it
is the target, otherwise the unique direct child in that scope. With no live
child, exactly one previously opened child is a redundant target; several
historical candidates refuse rather than selecting the latest. A never-opened
target refuses before producer commit. Closing or stopped targets do not
renew deadlines or cancellation. Commands are serviced in producer record
order, then canonical command ID order; a prior authoritative closure settles
requests for that target or its descendants. The bounded queue holds at most
1,024 pending requests per Worker, checked atomically with ordinary routes.

**Rejected:** targeted stop statements inside an Operator; restoring Graph or
Connector handlers; synthetic stop ports; arbitrary cross-scope control;
closing a freshly created instance to satisfy a missing target; using Loop's
run protocol as the instance lifecycle; a runtime-only pending-command queue;
weakening journal ownership checks to let one Executor close another directly.

## D-2026-09-09-1 — Bound-generated JoinExpired requires a destination

The opening-showcase audit exposed a disagreement with explicit destinations:
`within` and `last` can produce `JoinExpired` without an authored emission, but
the two productibility analyses missed that producer. Both now derive it from
the resolved Join activation. A missing disposition refuses with
`E-UNWIRED-OUTPUT`, including nested boundaries and forged packages. Without
a bound or an authored producer, the output remains dead.

Yoann approved the correction before implementation. It tightens acceptance
to the existing rule; runtime outcomes, v17 shapes, hashes of valid programs
and causal goldens do not change. No implicit discard or re-blessing is needed.

## D-2026-09-09-3 — Simulate the Contract without its physical Driver

Yoann confirmed the separation: an executable Connector Contract is required
to check and simulate a graph; its physical Driver is required only for real
execution. The Lab substitutes the external world, not types, pure request
framing, admission, reservations, exchange correlation or journal commits.
Pure Contract rules remain available without the networking Cargo features.
A port declaration alone is not an executable model of an exchange.

The existing source-injection syntax now connects `HttpServer` in `ack` mode
to the Lab: `listener.out = value` scripts a decoded request and a successful
fixed acknowledgement. It proposes a draft through the ordinary bound inbound
Driver interface; only committed acceptance supplies the Call reference for
`Completed { status: 202 }`. Pending admission retains the exact proposal and
withholds the acknowledgement. A sequential scenario unable to admit its
current request fails visibly rather than skipping it or reporting success.
`meta.ext` is authored fixture context, never inferred from a fake listener.
The source observation is the retained INBOUND, not a synthetic transaction.

Decoded-request simulation is graph evidence under those assumptions, not
evidence about wire decoding, authentication, body limits or delivery of a real
202. Those remain Driver conformance tests, including refusals before admission.
Source injection for `respond` and Session exchanges remains a named check-time
deferral until its scripted protocol is implemented. Unknown Contracts still
refuse; a missing physical Driver does not block an available Lab model.

This completes an existing Lab seam, outside the deployed graph. No artificial
App input, grammar addition, Contract id, plan-v17 movement, causal recipe or
existing golden change is required.

**Rejected:** making simulation depend on a live listener; pretending an
arbitrary mock payload defines a Contract; minting roots or terminals in a
fixture; changing the production graph solely to feed its tests; treating a
successful Lab scenario as qualification of the physical HTTP Driver.

## D-2026-09-08-1 — Loop extends the host terminal without contributing ports

Loop contributes no port. Operator and Graph use their declared `out` and
`err` as conventional Done terminals; other outputs are frames unless an
explicit `Loop.terminal` list replaces that convention. Productible exhaustion
requires a declared terminal union accepting `LoopExpired { last_state: Entry }`:
`err` by default, or the single output selected by `Loop.expired = <port>`.
An outbound Call instead extends its Contract terminal union; its frames and
Driver production guarantees stay unchanged. Neither terminal attribute can
override a Call's Contract. A proof-only Loop requires no exhaustion output.
An explicit fuel remains the cap when a measure coexists.

The compiler and independent validator resolve the output and its complete
type. The manager never chooses a port by its spelling. `LoopExpired` closes
the run; it cannot command that same run again. Driver completion still uses
the unextended Contract cases. Entry annotations propagate into `last_state`
before any observer formats it. Old `child.Loop.expired` wiring refuses by
name with the resolved host-output replacement; the attribute spelling is
not a port alias.

This amends D-2026-09-05-1 and the live Loop specification. The implementation
and qualification are tracked in
[the v17 ledger](../architecture/loop-host-terminal-v17.md). V16 is already
blessed: its reader is not silently reinterpreted. V17 adds the optional
resolved exhaustion output/type to LoopSpec, removes the contributed port,
and preserves all non-Loop semantic hashes and causal traces. Loop's r2
fact shapes, command identities and fold transitions remain unchanged; the
carrying emission supplies its ordinary port and recovery checks that port
against the deployed plan. No journal key or causal domain is repurposed.

The debugger derives admitted-application dials from the displayed journal
prefix, not a UI counter. Closed runs display zero. Pending re-admission does
not count as admission. Command dispositions remain text, not spatial edges;
r2 fact labels are presentation and never change sealed lifecycle triggers.

**Rejected:** another synthetic error port; implicit output declarations for
Operators or Graphs; accepting Driver-generated manager outcomes; a runtime
port-name convention; reusing blessed v16 with changed decoding rules;
counting an Advance as an Admit; inventing spatial routes for commands.

## D-2026-09-07-8 — Finish v16 migration and align the simulation debugger

R9 removes the active r1 Loop carrier/fold, completes explicit-boundary source
migration and blesses the three categorical packages once, after independent
decode and the enumerated [identity ledger](../architecture/rework-r9-qualification.md).
Echo and JSONL move with Process `@2`; gauge's semantic hash stays fixed.
Their causal trace goldens are unchanged. The parser AST snapshot moves for
source shape/spans, not as a second semantic or package version.
The old/new boundary-spelling twins were proven identical before enabling
`E-WIRING-MIGRATION`; full-envelope value `.` is unaffected. No old runtime
fold survives merely to keep a retired fixture green.

R9 follows the user's post-UI-review scope: the visual debugger controls only
checked simulations from disk. `run --ui` refuses before effects, with a named
real-I/O experiment gate. History is read-only backward/forward inspection;
Reset starts another scenario session, not a mutable branch. Branch replay
commands and the real-run UI controller are removed. This supersedes those
parts of D-2026-08-31-2 and D-2026-09-02-1, not normal CLI `run`.
Manager commands are not ports or PhysicalEdges. The UI exposes them and the
journal-derived Loop runs as dependencies/control details; only actual ports,
edges and mailboxes enter the spatial view. Loop state is identified by its
retained envelope reference, not re-exported through an unclassified payload path.

Code, protocol and deterministic layout tests do not substitute for visual
qualification. A real-browser/keyboard pass stays a named pre-public gate if
the browser controller is unavailable; no captures or visual pass are claimed.
The v16 semantic/core qualification is separate from that external UI gate.

Benchmark v1/v2 prompt cards remain frozen for old-wave attribution; a new v3
card describes current v16. Reference/referee pairs gain explicit types,
Contract terminals and source-specific observations while preserving their
business assertions. Canned errors remain intentional correction tasks.

**Rejected:** re-blessing causal witnesses without semantic cause; rewriting
frozen prompt cards; keeping r1 execution as test scaffolding; manufacturing
manager ports, simulation branches or another accounting/policy authority;
claiming full M1.9 visual qualification from layout tests alone.

## D-2026-09-07-1 — Preserve the embedded heap gate through the v16 cutover

The R4 review measured 130,772 B against the unchanged 131,072 B heap gate.
Before widening R5–R8, the same Cortex-M33 workload now stores each retained
journal packet indirectly: the record vector reserves 8 B handles rather than
216 B inline packets. Its measured peak falls to 105,988 B, recovering 24,784 B
and leaving 25,084 B (19.14%) of headroom. Record kinds, queries, payloads,
causal order, canonical bytes and history length are unchanged. The echo,
JSONL and gauge candidate ledger proves identity neutrality. The
[spike report](../architecture/spike-rp2350-emulated.md) owns the full numbers.

The heap gate remains **128 KiB**, flash **1 MiB**, and worst-case measured
relay **150 k instructions**. Embedded-path changes remeasure heap first.
Full Graph Loop and recovery qualification remain separate gates; unused
headroom is not evidence for unimplemented paths.

The reviewed unqualified cutover is checkpointed on `rework/v16`, leaving
`main` at its pre-cutover commit. Its red migration tests remain visible until
R9 gives each a named replacement or retirement; no ignore or blanket golden
blessing substitutes for qualification.

**Rejected:** raising the gate to absorb an accidental layout cost; deleting
retained history; claiming savings from the already-absent boundary inventory
or lazy r2 index; treating a candidate format commit as release qualification.

## D-2026-09-07-2 — Allocate retained timer schedules before blessing v16

R4 allocates journal **retention key 56**, `timer_schedule`, now, not when the
full R7 recovery reader lands. It is beside the accepting INBOUND or
TRANSACTION record, committed atomically with it, never an observation-stream
event or a mutable scheduler checkpoint. Its fixed shape is
`[armed_at_ns, [[kind, reference_bytes16, due_at_ns], ...]]`: signed logical
nanoseconds, existing timer kinds 1–9 (4 still reserved for Saga), and actual
arm order. Equal deadlines retain `(record ordinal, arm ordinal)` ordering;
renewing a reference does not sort away an arm. Empty schedules are omitted.
Negative logical instants are valid; an instant due before its arm refuses.
The payload, target generation and stale-wake fence remain derivable from the
accepting record's routes, instance, cause, Contract and Loop facts. This field
adds the missing time evidence, not an alternative authority for those facts.

This explicitly preserves D-2026-08-28-5/6: **key 56 is not in the causal
projection**. Key 37 continues to seal the existing `(kind, reference)`
obligations; key 54 alone seals observable `now()`. Storage must retain and
integrity-protect key 56 with its record even though the two-mode causal seal
excludes it. Neither package bytes nor program identity depends on these
per-execution dates. Physical record retention and causal projection are
different views; an export of causal bytes alone is not a recovery backup.

R4's typed carrier and strict canonical field codec cover the complete timer
registry. The executing r2 delay producer retains its schedule before commit;
the journal verifies agreement with the already-retained `Advance.due_at`.
Each Loop admission has exactly one matching arm: its run/iteration reference
cannot be renewed like an idle lifetime. Missing, contradictory or duplicated
Loop arms refuse before acceptance.
It is the first wired producer, not a claim of complete temporal recovery.
R7 must capture every join, lifetime/idle renewal, Closing, Call and handshake
arm at its accepting commit, and independently reconstruct the ordered queue.
Those profiles remain refused by recovery until their entire evidence path is
wired and tested. Missing retention is an explicit refusal, never
`restart_now + duration`. R7 needs no new field, timer tag or package bump to
use this representation.

**Rejected:** sealing raw scheduler dates and moving unrelated causal traces;
using sparse `now()` as a universal arm instant; reconstructing a deadline
from restart time; putting required recovery data in lossy telemetry; treating
field allocation as qualification of every producer or of a durable backend.

### R4 review confirmations

`nothing` is exclusive for a source case **across declarations**, as recorded
in D-2026-09-05-1; the audit fan-out belongs explicitly in each selected arm.
An impossible contributed output such as `Loop.expired` has no destination
obligation **and no dead-declaration warning**. `W-DEAD-OUTPUT` is reserved for
outputs actually declared by the author. This changes diagnostics only.

## D-2026-09-07-3 — Recover temporal obligations from their accepting prefixes

R7 wires key 56 into admission and emitting transactions for Call/handshake
delivery budgets, timed joins, instance lifetimes and renewals, and bounded
Closing. The same pure target-derived deadline selection is used before
commit and for post-commit installation. Recurring Call admissions also retain
their new request deadline. A record carrying observable `now()` must agree
with its schedule's arm instant. No new field, timer kind or package bump is
allocated.

Recovery constructs a candidate VM from the committed prefix, replaces the
live VM only on success, and appends nothing. A private admission-query seam
has two real implementations: the live Journal and a read-only prefix fold.
The latter prevents a later `l:stop` from moving an earlier delivery into a
successor generation. State deltas, exact routes, consumed cursors, timers,
close requests and residual stop consumption are checked before publishing
the candidate. Missing deadlines, changed Call facts, a Call opening before
its instance start, decreasing retained time and a resume instant earlier
than retained time all refuse atomically.

Outbound Call opens restore the held request without another `prepare`,
`start`, `poll` or `cancel`. Response closures restore their retained typed
completion; Timeout/Abandoned closures require the pure Contract projection
through `recover_with_bindings`. Consuming that completion is still a later
ordinary Connector transaction. The dispatcher treats old opens as already
started; only a genuinely new recurrence may launch a new effect. A queued
Call whose retained deadline has passed resolves as timeout before any new
Driver start, even when the caller drains immediately after recovery.

**Repository-contact correction:** fixed and tumbling lifetimes were armed at
`l:start`, contrary to `instances.md` §6.3's creating-admission rule; idle was
already armed at admission but its start record claimed the arm. All three
now retain and install their deadline at the accepting admission. A successor
waiting behind Closing keeps that original deadline; promotion does not grant
another lifetime. Its wake stays in the ordered scheduler until its instance
can start. The obsolete lifetime timer tuple on `l:start` is removed. This is
an intentional causal-trace change for timed-lifetime programs, not a claim
that every pre-cutover trace stays fixed. A single-pass start also implies a
recoverable close request if a crash interrupted the following commit.

Existing retained arms are never rebased onto restart time. New obligations
not yet committed are dated when their accepting transition actually commits;
the logical clock does not run backwards to recreate an unrecorded history.
In particular, key 56 does not establish a universal timestamp for all records
or reopen the deferred OTLP span claim.

The package, semantic hash and causal seal of the echo, JSONL and gauge
candidates remain unchanged; none of these witnesses has a timed lifetime.
The measured Cortex-M33 workload retains all 67 records and uses 111,452 B
peak, leaving 19,620 B under the unchanged 128 KiB gate. The
[R7 evidence](../architecture/rework-r7-manager-recovery.md) separates this
qualified recovery profile from still-gated Graph Loop contexts and
Event/inbound-Call/Session recovery. No golden is re-blessed.

**Rejected:** querying final generations during prefix replay; rerunning a
producer or an old effect to reconstruct its result; restarting a deadline at
resume/promotion; accepting missing timing as zero; manufacturing a Response
for scheduler-owned Timeout/Abandoned; silently claiming identity neutrality
for the corrected timed-lifetime traces.

## D-2026-09-07-4 — Keep window stop products in their live recipient generations

The R8 window probe exposed two defects in Graph-scope closure. A recipient
could stop before an upstream `on stop`, then restart with empty private state
to consume the final aggregate. Also, a reserved delivery to a leaf did not
authorize starting its new intermediate Graph under the closing parent.

At quiescence, a closing scope now retains every possible recipient of its
pending Operator stop products, including transitive ordinary execution paths.
Only after the producer stops and its products drain may those recipients
stop. Unrelated scopes do not wait. Inbound Call/Session input halves are
destinations, not fictitious paths to new external output roots; the same
Contract distinction is used by the spatial DAG validator. No new Graph
handler or application transaction is introduced.

A committed internal leaf reservation also proves admission of its intermediate
Graph scopes. The journal requires the exact producing envelope, compatible
canonical key/time-box frames, and the producer's retained parent-generation
chain at that commit. A reservation predating the intermediate owner's last
stop cannot be reused for its successor. A same-path foreign key or an
outside producer cannot authorize this opening under the closing parent.
These checks use retained routes and lifecycle facts, not a new record field,
address recipe, persistent index or package bump.

This corrects causal order and results for affected closing pipelines. It is
not identity-neutral for those runs: the spurious successor start disappears
and the final value uses the original recipient state. Nonaffected echo,
JSONL and gauge witnesses remain unchanged. The pure fixed/tumbling window
probe qualifies this path, including a lazily entered intermediate Graph and
recovery before/after closure; it does not qualify Graph Loop orchestration
or every bounded-Closing exchange profile.

**Rejected:** choosing stop order by owner spelling alone; reopening a
recipient with fresh state; accepting an ancestor start from lexical path or
business origin alone; granting a second generation the same reservation;
turning a Graph into an application Executor to emit its aggregate.

## D-2026-09-07-5 — Open Graph Loop only with retained admission and qualified obligations

The first executable Graph Loop profile is fuel-bounded and contains pure
Operator port applications. The manager retains one boundary admission before
fan-out, under the same 1,024-slot Worker bound as the other Loop hosts. The
Graph remains a scope, not an application Executor. Open and Admit are separate
commits; Admit consumes the exact FIFO delivery cursor and retains the selected
internal routes. A public App admission has its own ingress identity, never a
spatial self-edge. Same-origin admissions are distinct; internal fan-out is not
another run.

Contexts are derived from exact consumed deliveries and carried only across
routes that remain inside the corresponding instance. Next/Again/Done commands
and natural terminal crossings commit with their child transaction. A manager
consumes its command only after its other causal deliveries drain. Recovery
reconstructs the same admission, contexts, command and deadlines in a scratch
VM; producer transactions never rerun. Missing contexts/cursors, substituted
verdicts and consumption before sibling work all refuse independently of the
protocol-only fold. The selected Next type is independently checked against the
entry. Closing wins over delayed recurrence.

This is a qualified profile, not removal of all Graph gates. Nested/adjacent
Loops, Graph Calls, joins, lifecycle handlers, descendant-local stopping,
annotated recurrence, measure-only Graphs, direct descendant command statements
and boundary-only verdicts still refuse with their named qualification limit.
No assumption about absent effects, secrets or future work is inferred from
passing the pure-Operator tests. The complete R8 matrix and R9 stay open.

No v16 field or tag is added. The manager uses retained key 11 routing and
existing r2/timing facts; unused Graph indexes are absent from non-Graph-Loop
VMs. The three nonaffected candidate package/causal witnesses remain unchanged.
No golden is blessed. Exact tests, current failures and embedded measurements
are recorded in [R7 §8](../architecture/rework-r7-manager-recovery.md#8-pure-graph-loop-admission-and-quiescence--2026-09-07).

**Rejected:** opening one run per flattened child edge; using origin or lexical
membership as a delivery context; consuming manager admissions in hash order;
advancing while a sibling still owns work; resetting deadlines on recovery;
admitting unqualified classified/effectful graphs; adding another format bump.

## D-2026-09-07-6 — Qualify Graph Calls as complete causal obligations

The Graph profile of D-2026-09-07-5 now admits the existing finite outbound
Call Contracts: Process job, File read/replace and HttpClient. Every Call needs
a positive plan-constant timeout. The other profile limits remain: one Loop
owner, authored fuel, no Graph joins, nested Loops, classified recurrence,
descendant-local lifecycle or direct descendant command statements.

The independent spatial proof composes complete Call applications. It retains
each terminal case while selecting that case's routes and its correlated
frames. Never, exactly-one-on-Completed and zero-or-more are Contract facts,
not conclusions drawn from a mock. Arbitrary frame counts are checked through
the finite verdict algebra. Only terminal crossings of the same atomic
Connector transaction coalesce; downstream transactions remain distinct.
Recurrence and unsafe visibility are tracked together across the entire
application, including sibling Calls. An unsafe combination refuses as
`UnsafeLoopEffect` / `E-LOOP-EFFECT`. The source gate on compensable and
irreversible execution remains; raw-plan safety tests do not claim its removal.

A child Call's terminal settles that obligation, not the enclosing Graph run.
Only its selected output routes may produce the Graph verdict. The held
request, retained completion and downstream deliveries keep their exact run
context until consumed. A pending verdict blocked by such work is not ready
scheduler work. Another run's Call does not block it. Recovery re-derives both
the verdict and outgoing delivery contexts from the retained completion,
without starting an old effect or executing a producer again.

Two boundary distinctions were exposed by executable probes. Delivery timers
must use the same retained Graph-boundary edge identity as the delivery they
expire, not the executable edge projection alone. A keyed Loop Graph derives
its key from the manager's typed opening entry, not a descendant Call's wider
request type. Neither correction changes the meaning of a non-Graph-Loop plan.
Recovery retains a due cancellation as work; reconstruction itself does not
perform that Driver action.

No package version, wire key, Contract id, causal domain or golden changes.
R7 is still open; [the checkpoint](../architecture/rework-r7-manager-recovery.md#9-graph-calls-and-correlated-quiescence--2026-09-07)
records tests and the remaining gates. Nested Lab mock declarations remain
deferred: the Lab qualification uses an App Loop, and VM tests qualify nested
Call bodies through the same boundary Contract. UI work remains paused.

**Rejected:** independent response/exit choices which invent impossible
applications; one verdict per streamed frame; treating a child Call exit as
the Graph's Done; checking effects only at the command source; global rather
than run-scoped quiescence; rebuilding a timeout from restart time; recovering
by recalling an old Call; silently opening unrelated Graph profiles.

## D-2026-09-07-7 — Retain nested application contexts and exchange obligations through recovery

The Graph profile gates of D-2026-09-07-5/6 are extended by executable
qualification. Nested runs retain the exact enclosing context from Open.
Deliveries and held Calls carry it; a parent waits for that child run, including
its delayed recurrence, not unrelated work in the instance. Crossing a Graph
boundary removes its context from the outgoing delivery. An unconditional join
consumes candidates with one complete context and sees the nearest `Loop.state`.

A Graph made only of wiring needs no synthetic Operator. After admission its
manager records the verdict selected from the entry routes, then consumes it
under the ordinary quiescence rule. Production and consumption remain separate
crash points. Recovery re-derives the packet from the admitted state and static
routes, without re-emission or producer execution. No command becomes a port.

The integer measure fragment follows unchanged Graph entry state through
passive boundaries to its deciding Operator. The core independently re-derives
every decrease; transformed/opaque producers still need fuel. Annotation
propagation follows the same entry/command/boundary inventory, including
inherited state and expiry. Accounting meanings must agree after fixed-point
resolution; classification cannot disappear on Again.

The conservative Graph control proof refuses beyond 1,024 distinct outcomes
per combination instead of allocating an unbounded Cartesian product. Retained
or expiring joins and descendant lifecycle/lifetime decisions still need a
per-run completion proof outside this fragment and refuse at check. This is
not a claim that those programs cannot terminate.

Worker recovery reconstructs inbound Calls and Sessions, peer messages,
Session openings, pending writes and terminal cursors. Original handshake
deadlines remain facts; crossed/repeated exchange records refuse atomically.
It does not reconnect sockets or repeat committed/ambiguous I/O. An inbound
response already eligible for handoff is conservatively held until its original
transport supplies a terminal fact. Recovery cannot invent a missing transport.

**Identity:** no new wire key, record kind, Contract id or causal domain.
v16 keys 55/56 already carry these protocol and timing facts. Previously gated
profiles gain their specified facts; unrelated programs stay unchanged. R9
witness re-blessing remains separate and enumerated. Nested Lab mocks and
Executor observations resolve by exact path, outside plan identity.

**Rejected:** ambient state per instance; context reconstructed from `origin_id`;
waiting for another run; replaying a producer to recreate a command; projecting
a Session event as a fictitious Call; retrying because a physical handle is lost.

## D-2026-09-05-1 — Graph is structure; Executors transition; Loop uses durable manager commands

The pre-public Node model is revised before plan v16. A `Node` is either a
`Graph` or an `Executor`. A Graph owns a typed boundary, lexical children and
wiring, immutable owner values, instance, lifetime, Closing and Extensions. It
owns no application mailbox, mutable application state, handler or application
transaction. An Executor owns executable ports, a bounded per-instance
activation mailbox, journaled semantic state, atomic transactions, ordered
emissions and state deltas. `Operator` and `Connector` are Executors: an
Operator's transition is a checked DSL `Dispatch`; a Connector's transition is
defined by its versioned Contract and typed Driver events. They share
the commit machinery, not necessarily a public Rust trait.

Graph lifecycle remains explicit. The generic lifecycle controller still
commits `l:start`, `l:closing` and `l:stop` TRANSACTION records attributed to a
Graph owner. “Graph has no transaction” means no **application** transaction;
removing lifecycle records would erase M1.4's generation and containment
authority.

`Extension` replaces `Aspect`. It is attached to exactly one Node and may
contribute attributes, bindings, admission constraints, manager commands and
ports. `App` is the one static Extension of the root Graph. `Loop` is temporal
and admitted on an Operator, Graph, or finite outbound Call Connector profile.
`Saga` remains a future temporal Graph Extension. Extension order is
non-semantic and incompatible contributions refuse. `node::Type` disappears:
Graph and Operator are structural categories, App is an Extension, and a
Connector carries one exact versioned Contract identified by `ContractId`. The Connector attribute
`behavior = deterministic | probabilistic` is retained unchanged.
The current Rust/HIR name `ConnectorId` becomes `ContractId` at R3; this is a
source/API vocabulary change only and does not alter the validated id text or
wire representation.

### Dispatch and routing

Only Operators own handlers, with at most one handler per activation. A handler
contains recursive `Dispatch = Linear | ClosedMatch | OpenMatch`. Closed
matches operate on a tagged union, enum or bool; arms are disjoint, exhaustive
and canonically ordered by case. Open matches test predicates in source order,
first true wins, and `_` is mandatory. A match is the dispatch of a block or
arm, never a statement after a mutation or emission. Pure `val` declarations
may precede it.

On a one-port handler, `where P` is exact sugar for
`match { P => body; _ => nothing }`: false consumes the input and commits an
empty transaction. Both spellings lower to the same Dispatch before semantic
identity. On a join, `where` remains a pre-consumption admission guard: false
retains every candidate and commits nothing. `expect ... where ...` remains
scenario syntax. `when` on edges is removed.

A `RouteRule` maps a source's closed cases to canonical disposition sets. A
`PhysicalEdge` remains only a static port-to-port path with capacity; case tags
are not edge identity. Dispositions are a physical destination, the current
Graph's public/boundary output, a temporal-manager command, or explicit
`nothing`. Source sets expand to independent mpsc edges. Destination sets send
one envelope over N `DeliveryRef` values after one atomic capacity proof. Set
order is non-semantic. `nothing` cannot coexist with a real or manager
destination for the same case. The `.`/`.foo` boundary wiring sugar is removed
with a migration diagnostic; equivalent explicit wiring must retain the same
plan and semantic hash.

This exclusivity applies across separate declarations, so `nothing` always
means that the case has no destination and never changes meaning because of a
distant additive route. A common audit fan-out is consequently written in the
closed routing table itself:

```rastok
pay.exit -> match {
  Completed                  -> audit
  TimeoutExpired | DnsFailed -> { audit ; pay.Loop.again }
  _                          -> { audit ; failed }
}
```

Every possibly productible output case has an explicit disposition. An output
is exempt only when impossibility is re-derived from syntax, structure, closed
typing, the Connector's resolved Contract, or whole-Graph unreachability. Unknown is
possible. Missing disposition is recursive `E-UNWIRED-OUTPUT`; deliberate
discard is `-> nothing`; an impossible declaration receives
`W-DEAD-OUTPUT`. `RootSink`, `UnwiredPolicy`, `vm.errors()`, the name test for
`err`, and the second `node: None` observation disappear. A Driver which emits
a contractually impossible port or case raises `ContractViolation` before the
value is published.

Statements and emissions remain transaction-atomic and source ordered.
`(a, b) = value` evaluates the right side once and creates two distinct
envelopes in written target order. `(a, b) = (v1, v2)` evaluates values
left-to-right and pairs them. This differs from `source -> { a b }`, which
creates one envelope and two deliveries with canonical unordered destinations.
All involved capacities are proved before commit.

### Loop r2

Loop recurs on the host's existing entry interface. The recurrent entry is the
unique input, otherwise `in`, otherwise required `Loop.entry`. A distinct
opening admission, not merely `origin_id`, identifies every run:

```text
LoopRunRef = derive(rastok.loop-run.r2, InstanceRef, OpeningAdmissionRef)
```

Routed admissions carry a journaled stack of active LoopRunRefs. This
`LoopContext` is causal route metadata, not envelope data. It disambiguates
fan-out, permits several runs in one instance, binds `Loop.state`, scopes
quiescence and recovery, and makes nested Loop masking exact. A join inside a
Loop application accepts only candidates with the same context.

Loop contributes qualified attributes `Loop.fuel`, `Loop.delay`,
`Loop.measure`, `Loop.entry`, `Loop.terminal`; binding `Loop.state`; commands
`Loop.next`, `Loop.again`, `Loop.done`; and the sole added output
`Loop.expired`. Commands use destination-like syntax but are neither ports nor
PhysicalEdges. `Next` supplies the next entry envelope, `Again` retains the
complete current state and uses its source only as cause, and `Done` closes
without a business envelope. An Operator may issue a command as a statement;
an output may route to the owner or a child's qualified command.

A command has two durable phases. `CommandProduced` is part of the producing
Executor transaction; `CommandConsumed` is part of the later manager
transaction. The pending bounded inbox is the journal fold of produced minus
consumed/invalidated commands. This distinction is mandatory: otherwise a
crash after producer commit but before manager work could neither restore the
command nor avoid replaying the producer. A manager is attached runtime
machinery, not a Node or port, and shares the owner's address and generation.

Each application selects exactly one verdict: `Next`, `Again`, `Done`, or
`Stopped`. `nothing` is not a verdict. For Operator/Graph, one ordinary output
is terminal by default and several require canonical `Loop.terminal`; no
output requires explicit `Loop.done`. Several terminal outputs in one
transaction coalesce to one Done. For an outbound Connector Call, frames are
non-terminal and the sole `exit` completes the application. An exit-case or
earlier frame command overrides natural Done; disposing or business-routing
the exit does not. `Loop.terminal` is therefore invalid on Connector Call.

`Loop.fuel = N` counts applications, first included. Next and Again consume one
unit. A recurrence requested by the final permitted application produces no
re-admission: the manager emits `Loop.expired` with
`LoopExpired { last_state: EntryType }`. The manager retains the full state
envelope; `last_state` is its data. `Loop.expired` obeys ordinary explicit
routing and is visible only to the parent in v16. On a root `App + Loop`, it is
itself a public App-boundary output. A measure-only proof makes it
impossible; when an explicit smaller cap also exists, it remains conservatively
possible unless termination within that cap is proved.

The manager waits for run-scoped quiescence before consuming a verdict. An
Operator application completes at its Dispatch commit, a Graph application at
contained causal quiescence, and a Connector Call application at its terminal
exit. Work outside the host or belonging to another run does not block.
Closing invalidates commands and wakeups and always wins. Delay remains logical
timer kind 5, with no future destination capacity held while waiting.

Every outbound Call terminal occurrence is classified conservatively as
`before_effect`, `outcome_unknown`, or `after_effect`. Next/Again after
`before_effect` is allowed; the other classes require
`effect = none | idempotent`. `compensable` does not make forward repetition
safe. Process job, File read/replace and HttpClient are v16 Loop hosts;
HttpServer, both WebSocket Session contracts, Process service, Event and inbound
Call are not. An opaque Connector requires positive fuel.

World-originated Driver facts still enter as INBOUND roots. A response
correlated with an already-open outbound Call or Session write does **not**:
it enters the bounded Connector activation path, settles the existing OUTBOUND
obligation and supplies a Connector transaction. Plan v16 does not invent a
second root for it.

### Identity and staging

Plan v16 is one categorical bump after source/HIR stabilization. It carries
`ExtensionSpec`, recursive Dispatch, RouteRule, canonical sets, manager
commands, productibility, Loop r2 and exact Loop context; retires Graph
handlers, ChildPort, old Loop `next`, `unwired`, RootSink, and old Aspect
transport; and never reuses a tag. The owner table, envelope/value/type
registries, three journal record kinds, timers, instance/lifetime/Closing,
Connector roles/exchanges and the Connector `behavior` axis remain.

Removing `unwired` while adding v2 production/terminal/effect-stage contracts
moves the six executable Contract identities to `rastok/{process,file,http-client,
http-server,websocket-client,websocket-server}@2`. V15 and all `@1` plans refuse
rather than being reinterpreted. Existing key 53 and the
`rastok.aspects.r1`/`rastok.loop.r1` domains remain historical; r2 facts use new
append-only keys/domains.

R0 and R1 are specification/architecture only. R2 realigns the live corpus and
then stops for review. Source/HIR work begins at R3, the single package bump at
R4, explicit destinations at R5, routing/Dispatch at R6, manager/recovery at
R7, host qualification at R8, and global migration/UI realignment at R9.

**R3 source/HIR precision (2026-09-05).** The accepted whitespace-separated
set spelling, `source -> { a b }`, survives; the EBNF's formerly mandatory
`sep` between set members was stricter than this decision's own examples and
is corrected to optional explicit separators. This does not relax match-arm
separators. A Dispatch path is bounded to 32 match decisions, including a
single-port `where` after desugaring. This is a source/HIR resource bound;
R4 independently validates Dispatch and total package depth.

The `.`/`.foo` wiring migration diagnostic is specifically an R9 gate. R3
continues to accept that spelling silently while v15 compatibility and the
equivalent explicit-boundary witnesses remain under test. This staging does
not weaken the final removal rule. The parser conformance snapshot changes
from `HandlerDecl.body` to `dispatch: Linear { statements, span }`; this AST
movement is recorded separately from the three semantic/package/causal
registries in the R0 identity ledger.

### Supersession ledger

Older entries remain historical records. The following table says exactly
which clauses cease to be live; unlisted clauses survive.

| Decision | Treatment in v16 |
|---|---|
| D-2026-08-27-4 | `Kind × Type × Aspect` is superseded by Node / Graph / Executor / Connector / Extension. Singular `instance` and ordinary reducer/aggregator/window/join patterns survive. Each Connector owns one versioned Contract; Contract is not another Node category. |
| D-2026-08-28-1 | Graph-only Loop ownership and the `next` port are superseded. Fuel, delay, measure, finitude, qualified Extension attributes, `LoopRunRef`, journal authority and no-hidden-clock survive. |
| D-2026-08-28-2 | Graph handlers, `HandlerOwner::Graph`, `ChildPort` and their v13 transport are superseded. Fail-closed staging and Registry-free validation survive. |
| D-2026-08-28-3 | “Every ordinary output terminates” is replaced by resolved `Loop.terminal` plus exactly one `Next | Again | Done | Stopped` verdict. Verdict uniqueness and the distinction from instance `stop` survive. |
| D-2026-08-28-4 | Port emission to `next` is replaced by a durable manager command and temporal re-entry at the remembered host input. Fuel and the journal fold survive. |
| D-2026-08-28-6 | Direct-child `ChildPort` activation is superseded. Timer kind 5, no destination reservation during delay, and Closing invalidation survive. |
| D-2026-08-28-7 | The proof is re-expressed over Dispatch and manager commands rather than Graph handlers. Integer ranking, re-derived certificates and the independent `no_std` verifier survive. |
| D-2026-08-28-8 | Passive-Graph flattening and the stated limits of nested Loops survive; host qualification broadens in v16. |
| D-2026-08-18-1 | Graph-handler clauses are superseded. Instances, lifetime, address, placement, Closing, state ownership and join `where` survive. |
| D-2026-08-14-1 B5/B6 | `RootSink`, `UnwiredPolicy` and implicit output disposition are superseded. Typed ports and ordinary `err`/`exit` names survive. |
| D-2026-09-02-1 | UI projections derived from Graph handlers or port-shaped Loop controls are superseded and must be rebuilt at R9. Its retained-history and simulation-only branching decisions survive. |
| M1.8 decisions | Unchanged. Schema policy, secrets, privacy, observations and cost remain orthogonal to this rework. |

D-2026-08-28-5 (`now()` and explicit logical time) is unchanged. Fuel, proof,
instance/lifetime/Closing, typed ports, DAG delivery, and fail-closed validation
otherwise survive as restated in the live specification.

**Rejected:** making Graph a disguised Executor; a public `Executor` trait
without a substitution need; treating commands as ports/back-edges; deriving a
run from origin alone; one fold fact which conflates command production and
consumption; implicit output sinks; treating declaration order of sets or
closed matches as semantic; using `nothing` as a Loop verdict; retrying a
compensable/irreversible effect because compensation may exist; minting an
INBOUND root for an outbound response; and rewriting historical M1.7 reports to
hide the supersession.

## D-2026-09-02-1 — UI history is exact; only simulation may branch from it

The debugger names three different positions instead of overloading “step”:
`A<n>` is an authored scenario action, `B<n>` an atomic retained UI boundary,
and `R<n>` a causal journal-record index. Universal lazy lifecycle transitions
remain visible `B` boundaries. `Play`, manual `Step in` and `Continue` use the
same execution transitions; Play merely presents every retained boundary at a
readable client-side cadence, while Continue selects the fast path.

Before publishing its replaceable newest-live notification, the execution
owner retains the complete redacted read-model frame for every `(branch,
boundary)` in memory. That ordering prevents a fast engine or slow browser from
creating holes in the timeline. The frame includes its journal prefix, VM
inspection, observations, logical time, Lab position and CostReport. It is a
disposable projection over existing authorities, never another journal, and is
discarded with the process.

For `run`, selecting an older boundary changes only the browser view. The real
machine continues at its live head and external effects are neither replayed
nor undone. Historical graph, envelopes, instances, private state, mailboxes,
Calls/Sessions and accounting are read-only; only global safe Pause and Close
remain available. `Return live` restores the live projection. This is temporal
inspection, not execution rewind.

For `simulate`, `Branch from here` is an explicit deterministic reconstruction.
The owner creates a fresh checked scenario and replays the selected branch's
authored actions, VM transactions and Driver polls. It then
requires exact equality of causal records, VM inspection, observed envelopes,
logical time, action cursor, quiescence and failures at the requested boundary.
A mismatch refuses without changing the current scenario. A match creates a
new branch at that boundary; its source branch and journal prefix remain
immutable and remain selectable for inspection. Further steps occur only on
the new branch. Reset and scenario selection similarly start a fresh
branch rather than truncating old history; each replay recipe carries its own
checked simulation/scenario identity.

The compound graph keeps a deterministic crossing-reduced lane plan as hidden
placement authority, but renders each complete route as a smooth cubic spline
with horizontal port tangents and compact arrows. Shared sources and targets
receive distinct approaches before the visible curves can coincide. The root
Graph grows to hold a complete result gutter; that gutter never steals width
from its Nodes. The complete drawing then fits the available canvas with a
24-pixel viewport margin and a maximum 1.5× presentation scale. Qualified
result labels, edge badges and port labels retain their minimum clearances in
drawing units, independent of that viewport scale, while SVG text remains at
the surrounding interface's screen-space size. Only retained topology edges
with an inspectable destination become visible pipes. Unwired outputs and
explicit `-> nothing` drains remain local square ports, with no terminal cap
or synthetic sink. A separate thin dotted control layer is derived from the
validated Graph-owned handler table: it connects an activation port to each
boundary output that one of those handlers may select, carries a compact `on`
marker, and has no mailbox or envelope animation. `next` is distinguished as
temporal control, while manager-produced `expired` begins at an explicit Loop
manager source. Selecting a relation opens the owning Graph's source-faithful
definition. This explains Graph results without falsifying them into spatial
DAG edges. Active
owners show an `⧉ n` instance count; selecting it opens
the structured instance inspector. Selecting a box lightens its body and
border without changing any port color. An input edge badge sits toward its
receiving square port, reads `≡ n`, and contains only current
mailbox occupancy and opens the exact redacted FIFO. Causal traversal counts
remain in history and never masquerade as live queue contents. The canvas may
be panned and zoomed, and a physical splitter resizes its inspector without
changing graph geometry or execution state.

This is a package, semantic and causal non-move. Branch numbers, boundary
ordinals, retained frames, playback speed and viewport geometry are physical UI
state and enter no plan, semantic hash or journal recipe. No graph-layout,
timeline or replay dependency is added.

**Supersedes in D-2026-08-31-2:** the user-facing `Step N` vocabulary,
instantaneous Play presentation, fixed intrinsic graph sizing, short curved
turns, leader-linked edge badges, synthetic unwired-output routes, cumulative
mailbox labels and browser-side injection editing. A pending checked injection
is read-only evidence from the `.rkg` scenario; `App.in` is topology, not an IDE
affordance. The atomicity, safe-pause and no-hidden-clock rules of that decision
remain unchanged.

**Rejected:** mutating a live `run` from historical state; truncating a
simulation journal in place; reconstructing history from topology alone;
dropping intermediate frames when SSE coalesces; hiding lifecycle boundaries;
throttling the semantic engine to animate; exposing raw keyed values as graph
badges; shrinking Nodes to donate a fixed output gutter; rendering coincident
visible routes; drawing a destination for `nothing`; leaving handler-produced
Graph outputs visually indistinguishable from unused ports; and
adding a licensed third-party graph layout runtime. A rich terminal debugger is
deferred as a separate projection over the same controller/read model; neither
its CLI spelling nor a second execution semantics is introduced here.

## D-2026-08-31-2 — `--ui` is a capability-protected journal debugger

The M1 live visualization is named `--ui`, not `--watch`: watch is already a
deferred File profile and commonly means source reload. The option applies to
`run` and `simulate`; `check` keeps its direct textual result. Every UI session
starts paused and advances at the VM's existing atomic transaction boundary.
Simulation additionally exposes its authored action boundary, so `inject`,
`advance` and `expect` never masquerade as Node transactions. Usability
evidence refined the primary simulation boundary advance: at a quiescent boundary it
applies exactly one authored action; otherwise it executes exactly one ready
VM transaction. It never combines both in one click, while the specialized
controls retain the distinction for teaching and diagnosis.

Retry belongs only to a VM transaction refusal whose work remains reserved and
whose failed attempt committed nothing. A Driver/polling failure may arrive
after an earlier transaction committed; the controller reports that distinct
state, preserves the history and refuses retry instead of replaying committed
work.

Pause is a safe-point request. New external admissions and handler firings
stop at the transaction boundary; already-open outbound Calls remain polled
until their typed terminal outcomes are retained, then wait unconsumed for the
next boundary. Long-lived Sessions stay open and rely on their existing bounded
transport/mailbox backpressure. While `Paused` or `PauseRequested`, the hosted
wall-to-logical mapping is frozen: physical time spent inspecting the run does
not age Call timeouts, instance lifetimes or delayed Loop continuations. A
resume rebases that mapping at the frozen logical instant; retained external
outcomes remain facts and are not rewritten. A real run may retry an
uncommitted fault but cannot reset effects already visible to the world;
simulation reset creates a fresh complete world.

The hosted page is embedded and self-contained behind an independent Cargo
feature. It binds loopback only, uses an OS-random capability on every request,
allows one controller plus read-only observers, and never opens a browser
automatically. The default port is OS-selected; `--ui-port` requests one exact
port and fails before execution if unavailable. Minimal and embedded images
omit the entire physical capability while reading the same plans.

The journal remains the only causal authority and `CostReport` remains the
only accounting fold. Snapshot/SSE state, controller ownership and rendering
are disposable hosted projections. The UI shows the full in-memory history,
validated topology, logical instances, Node-private state and materialized
DATA; it never forces decoding. `@sensitive` and `@personal` paths redact
before projection and `@secret` never renders. An unclassified materialized
scalar discloses its content beside `kind` and `materialized`; an unmaterialized
encoded root still discloses only representation metadata and known size. No
commit time or duration is invented.

The topology view is a compound graph derived from the validated lexical owner
table: the root `App` frame contains its direct Nodes and nested Graph frames,
recursively. It never guesses containment from dotted paths. Operators remain
green and Connectors blue, but compact cards carry only name, NodeType and
NodeAspects; the former circle/square/cross seed glyphs are superseded. A hollow
square is an input port and a filled square is an output port, and exact edges
attach to those pins. A Graph port keeps its declared side but may slide vertically
and exchange display order with peers: deterministic barycentric sweeps place
Node cards and side pins jointly, then interval-colored orthogonal tracks avoid
collinear overlap and minimize remaining crossings before compacting bends and
length. A flattened cross-hierarchy edge passes through a nested Graph pin only
when the retained port name identifies it uniquely (or that side has one port);
geometry never guesses an ambiguous composition seam. The implementation is an
original dependency-free hosted asset, not a linked graph-layout runtime.
Rounded corners and smaller terminal arrows remain presentation only. A deterministic
obstacle scorer places count badges beside, never deliberately over, route
segments and keeps them clear of Nodes, Graph headers, other edges and prior
labels; a short leader preserves association. Port names occur once at their
owning pins with a background halo rather than once per edge. Each routed input
badge contains only current live mailbox occupancy, which falls when work is
consumed; selecting it opens the exact redacted FIFO. Causal route history stays
in the journal/history view. The last-boundary highlight is separately physical: an
authored Lab action names itself and says that no Node
fired, while a transaction names its exact owner and trigger. Lifecycle
transactions remain visible instead of being skipped to manufacture a more
dramatic animation. The read model exposes both their exact trigger and a
human summary (`Instance opened`, `Instance closing`, `Instance stopped`), so
universal lazy `l:start` facts cannot be mistaken for duplicate deliveries or
port-handler transactions.

Every authored action or scheduled VM transaction receives one disposable
boundary ordinal. D-2026-09-02-1 names it `B<n>` and separately names authored
actions `A<n>` and causal records `R<n>`. Play and repeated Boundary use the
same transition and must retain the same causal sequence. Exact admitted,
consumed and emitted routes animate one envelope marker, while route-less
lifecycle or assertion work pulses its truthful owner/banner. Call and Session
boundary facts name and pulse their Connector and move a small marker in the
truthful outbound/inbound direction. These animations are never journal or
scheduling facts and disappear under reduced motion.

A Boundary requested from `Paused` returns to `Paused` after exactly one boundary,
even if it opens a Call. That Call remains inspectable and later scripted
`expect`/`advance` actions remain reachable; entering `PauseRequested` here
would strand a deliberately silent Call while logical time is frozen. Only an
explicit Pause requested from Running waits for already-open Calls to reach
their safe retained outcome. The browser disables transport controls from
command acceptance until the next snapshot revision, so rapid clicks cannot
collapse or queue ambiguous user-visible boundaries.

Selecting a Graph or Node makes its exact authored declaration, including
attributes, handlers and guards, available without forcing the Definition tab.
These excerpts live in the compiler's
physical debug side table: the browser neither parses source nor reconstructs
it from the plan, and a stripped deployment may omit them without changing
execution. The same disposable selection filters history, instances,
Calls/Sessions and cost rows to the selected Node or lexical Graph subtree;
clearing it restores the complete retained projection and never changes its
authority.

The pedagogical surface follows an envelope from boundary to boundary without
splitting a transaction. The compact sticky bar always names `Rastok UI`, the
execution mode and state; transport controls use labelled icons and the page
follows the host light/dark preference. Generic operating instructions are not
embedded in the page. Graph ports, including the root App ports, sit on their
owner frame, and the selected envelope is presented as the two normative
zones `META` and `DATA`, never as an ambiguous UI-only “payload”.

The next authored `inject` is a read-only typed projection before `App.in`.
`DATA`, `meta.ext`, sequence and provenance remain authored on disk;
`meta.sys` does not exist until ordinary admission mints it, and an admitted
envelope is immutable. The UI has no ad-hoc action, edit command, internal-edge
mutation or second ingress path. This deliberately keeps M1.9 a debugger rather
than beginning an IDE.

This is a package/semantic/causal non-move. Port, capability, browser state,
view revisions and physical timing enter no identity recipe. A later UI crate,
remote administration, hot reload, breakpoints, state mutation, durable
history and OTLP each require their own decision.

**Dependency decision:** the feature reuses the already pinned Hyper/Tokio
HTTP/1.1 server boundary. The standard library has no portable CSPRNG, so the
feature directly pins `getrandom 0.4.3` (Rust 1.85; MIT OR Apache-2.0; already
present transitively in the all-feature lock) solely to mint the local control
capability. Its public seam is an opaque token; removing or replacing the
crate changes no Rastok protocol or identity.

**Rejected:** a Git feature branch; `check --ui`; binding beyond loopback;
unauthenticated localhost controls; all tabs racing commands; auto-opening a
browser; starting live before the controller explicitly resumes; letting
debugger suspension consume logical timeout or lifetime budgets; pausing by
cancelling Calls or Sessions; real-run reset; a second journal/accounting
store; forcing payload materialization; post-format regex redaction;
free-form/manual envelope injection; editing source-authored actions, `DATA`,
`META` or committed envelopes; stepping individual edges inside one atomic
transaction; and reusing `File.watch` vocabulary.

## D-2026-08-31-1 — Executable clear-secret schemas refuse at source check

An unused named schema may describe `@secret` data because the declaration is
structural vocabulary and does not enter an executable plan. Every use of that
schema at an executable owner — Graph or Node port, owner `val`, Node `var`, or
handler `let` — refuses with the named `E-CHECK-DEFERRED` gate until
field-level externalization can replace clear material with a reference before
causal admission. This is deliberately stronger than waiting for a decode or
VM boundary: M1 has no deployment capability under which such a source
program could execute, so producing a plan would violate the rule that every
not-yet-executable construct refuses during checking.

Categorical validation continues to accept and prove the closed annotation
vocabulary, and VM boot retains `UnsupportedDataPolicy` as defense in depth
for a decoded or otherwise forged plan carrying clear-secret policy. That
defense is not a source-language execution path. When field externalization
lands, this one checker gate becomes a capability judgement; the package and
runtime defenses do not need a second policy model.

`env()` and `secret()` are reserved boot-reference functions, not unknown
expression functions. Outside a declared compatible Connector setting their
diagnostic names the restriction and the two M1 sites (`HttpClient.url` and
`HttpClient.auth_bearer`). This changes only refusal timing and diagnostics:
no valid package, semantic identity, causal record, or registry allocation
moves.

**Rejected:** compiling a source program that every current deployment must
reject at boot; erasing `@secret` to make it executable; treating
`secret()` as an unknown spelling; and removing the package/VM belt merely
because the source checker now owns the ordinary path.

## D-2026-08-30-6 — CostReport is an exact per-flow journal fold

M1.8 A7 derives accounting observations lazily from causal envelopes and the
independently validated effective annotation tables. It does not add an eager
metrics store to the VM or journal commit path. Each observation retains its
A5 `RecordObservation`, exact envelope id and full flow point: origin,
instance generation, owner, Node, Connector, port and typed Call/Session
correlation. A deployment price profile may add provider/model dimensions;
those are attested deployment facts, never inferred from a string in payload
data. Reports aggregate only equal complete flow points. They expose no
misleading graph-wide total which would count an exact-copy measure once at
each port it traverses.

Dynamic-codec Connector ports remain `variant`, so a declared measure may
first become structurally visible in the typed decoding Operator immediately
after the boundary. In that case the report follows the sealed envelope-parent
chain, bounded by the current committed record, to the nearest causal
Connector producer. A later record can never explain an earlier observation.
Exact copy/projection retains
the annotation and this attribution; a calculation already drops accounting
meaning under A3 and therefore cannot inherit a stale provider. The current
Node and port remain separate flow dimensions. No topology search, name match
or “only Connector in the graph” fallback is allowed.

The pinned measure registry now owns carrier, exact internal unit and subset
relation. GenAI cache-read and cache-creation tokens are subsets of inclusive
input tokens; reasoning tokens are a subset of inclusive output tokens. Every
sample remains visible. Pricing carves out a subset only when that subset has
its own matching rule, then prices the inclusive remainder separately. A
contradictory subset greater than its total refuses. Without a rule, a
positive remainder becomes typed `unpriced`; it is never a zero monetary row.
An explicitly declared zero unit price remains distinguishable and does
produce an exact zero calculated cost.

All monetary arithmetic uses normalized exact decimals and one pinned ISO
currency. `Money` remains the canonical struct convention, not a `DataType`.
Unit-price denominators are positive powers of ten, so division is represented
by decimal scale. A rule or result which exceeds `decimal(38,18)`, overflows or
requires rounding refuses. Provider-reported amounts and
pricebook-calculated amounts retain different evidence. Costs are aggregated
only when flow, basis, currency, priced measure and price-profile evidence all
agree; there is no cross-currency total or implicit FX rate.

The optional hosted deployment `cost_profile` is a closed list keyed by exact
Connector path and registered measure. Each row fixes provider, model/SKU,
units, decimal amount and currency. Rules for one Connector must agree on its
provider/model dimensions; duplicate Connector/measure rules refuse. The
profile has a stable reference and a digest over its canonical typed content.
An annotated `pricebook_calculated` field has that evidence only when the
profile contains a row for its sealed causal Connector; an unrelated or empty
profile cannot attest the amount.
Only that reference/digest enters the immutable run-header attestation. A
profile supplied while replaying a run must match the attestation; changing a
profile starts different deployment evidence but never changes graph identity.
An already typed accounting stream also carries those provider/model
dimensions and refuses if a caller attempts to reprice it with a profile that
names different dimensions.

The public `CostReport` can also omit an attested profile deliberately, in
which case exact usage remains and calculable quantities become `unpriced`.

Annotated encoded bytes without a materialized logical value refuse reporting
rather than disappearing or being decoded by an accounting-specific codec.
The initial executable field measures are non-negative. Missing, negative,
malformed, overflowed and unattested facts all stay distinct typed refusals.

A7 allocates no package tag, semantic extension, causal-record key or golden
move. The profile attestation is non-causal run metadata. Unannotated programs
still take the empty-table fast path; the M33 relay path performs no accounting
walk unless a caller explicitly requests a report.

**Rejected:** exporter metrics as accounting authority; floating-point money;
implicit rounding or FX; summing cache/reasoning subsets into their inclusive
parents twice; silently dropping undecoded annotated data; treating an
unknown price as zero; one global total across copied flow points; and putting
deployment prices in graph/package identity; pricing a dynamic-codec value by
an unsealed topology guess instead of its causal ancestry.

## D-2026-08-30-5 — Hosted OTLP waits for exact temporal observations

M1.8 A6 qualifies but does not add a hosted OpenTelemetry dependency. The
candidate official stack observed on 2026-08-30 is `opentelemetry 0.32.0`,
`opentelemetry_sdk 0.32.1` and `opentelemetry-otlp 0.32.0`. All three declare
Apache-2.0 and Rust 1.75. The intended minimal transport is OTLP/HTTP protobuf
with a blocking client confined to a disposable exporter thread; default
features, logs and an application-wide async runtime are not part of the
candidate. The official Rust status page still labels traces, metrics and logs
Beta, and the 0.32 SDK retains semver-moving exporter and processor APIs.

A5 deliberately refuses to assign exporter wall time to a causal commit. Its
structural observation therefore has exact correlation but no `committed_at`
or duration. Constructing SDK spans now would either invent those facts at
projection time or add a second eager hot-path record which the Cortex-M33
gate has already rejected. Neither is an acceptable qualification result.
OTLP integration resumes only after a retained observation owns the exact
temporal facts it exports; it must then record the exact selected dependency
features and transitive tree, binary delta, bounded-queue behavior, shutdown
behavior and an independent hermetic Collector transcript before landing.

This is the calendar-safe deferral authorized by A6's exit criterion, not an
OpenTelemetry substitute. No crate, feature, CLI flag, package byte, semantic
hash or causal record changes. JSON logging and the deterministic report plane
remain executable; M1.8 proceeds to A7. The removal strategy stays trivial
until the future gate: OTel types may exist only in one optional hosted module,
behind the existing typed `RecordObservation` seam.

Primary qualification references:

- <https://opentelemetry.io/docs/languages/rust/>;
- <https://opentelemetry.io/docs/languages/rust/exporters/>;
- <https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry-otlp>.

**Rejected:** timestamps sampled by the exporter; zero-duration spans
presented as measured transactions; a dependency landing without an
independent Collector; OTel types in `rastok-core`; and delaying the exact
`CostReport` foundation merely to ship an optional transport first.

## D-2026-08-30-4 — Retained observations and disposable logs are different planes

M1.8 A5 adds one typed `RecordObservation` beside every committed causal
record. The observation names the exact record transition and carries only
correlation derivable from causal history: `origin_id`, logical owner and
instance, Connector path, and a direction-safe outbound/inbound Call or
Session/write reference. It never contains Host, Worker, socket, thread or
exporter identity. The observation stream has the same order as its causal
records, so it does not duplicate a mutable or host-width record ordinal. Each
coordinate is optional when that individual causal record owns no direct fact;
the projector never guesses one from placement or arrival time.

The retained stream is accounting input and belongs to `Journal` even though
it is excluded from causal encoding, replay decisions and trace fingerprints.
`Journal::retained_observations` is a semantic query returning owned values;
its suffix form accepts the consumer's semantic stream offset so an exporter
does not rescan or reformat the complete prefix on each pump. A durable
implementation may store, index or reconstruct observations without exposing
a physical record layout. `MemJournal` reconstructs structural correlation
lazily from the immutable record prefix. That keeps the default and embedded
commit path byte-for-byte and allocation-for-allocation unchanged; only a
report or exporter pays the projection. Correlation for a record sees only
earlier history, never the record being summarized or a future fact.
The existing VM `Observed` delivery log remains a Lab/debugger surface with
payloads and is not renamed or reused as accounting authority.

Hosted structured logging consumes only copies of that retained stream. One
bounded synchronous channel separates a never-block `try_send` producer from
the possibly blocking exporter thread. A full or disconnected channel drops
the projection, advances the exporter cursor and saturating-increments one
counter; it cannot create retry backlog, fail a transaction or change the
journal. `--json-logs` opts the standalone runner into this lane. Payload is
absent by default. An explicit payload projection consults the resolved sparse
annotation table and applies `RedactedValue` before JSON construction;
classified encoded data becomes one marker rather than being decoded by the
presentation layer. Empty annotation tables take the direct zero-walk path.

A5 adds no categorical field, semantic extension or causal record key. Every
existing package, semantic witness and causal trace therefore stays
byte-identical. Exact committed-time, duration, measure and monetary
observations extend this typed plane in A7 (and the optional A6 exporter)
instead of being guessed from projection time.

**Rejected:** an untyped string map; logs as the only copy of accounting
facts; blocking or retrying on exporter pressure; making log loss a VM error;
reusing the payload-bearing Lab observation log; including payloads by
default; post-format regex redaction; and assigning exporter wall time to a
journal commit.

## D-2026-08-30-3 — Deployment binds references and privacy before effects

M1.8 A4 opens one deliberately narrow boot boundary. `env("NAME")` and
`secret("logical/path")` are exact root expressions with one string literal;
they cannot be combined with another expression or hidden inside a
calculation. The checker admits them only where the Connector contract has a
matching Registry-free policy. The executable M1 surface is
`HttpClient.url = env(...)` and `HttpClient.auth_bearer = secret(...)`.
Every other setting, unit/App boot value, incoming secret field and composed
boot expression still refuses until its real and scripted consumer lands in
the same gate.

The compiler lowers each reference to a reserved categorical expression path
under `$boot`. That spelling is unforgeable from source and already fits the
v15 expression transport, so A4 adds no package field or version. The logical
reference and selected boot site are semantic graph identity. Environment
values, secret material, provider coordinates, privacy profiles and CLI paths
are deployment facts and never become graph identity. Every non-secret boot
value is nevertheless fixed by its canonical tagged-value digest in the
immutable run header; a changed endpoint cannot silently enter replay merely
because it was outside graph identity.

`rastok-core` owns only typed `DeploymentProfile`, `BootBindings`,
`SecretRef`, `SecretProvider` and privacy-attestation values. It parses no
human configuration and performs no environment lookup. The hosted runner
accepts one closed JSON profile through `--deployment <profile.json>`.
Non-secret environment values may occur in that profile. A secret entry
contains only a logical reference, an environment-provider coordinate and an
immutable declared version; the material itself comes from that environment
coordinate and a matching `<NAME>_VERSION` variable. Secret material is never
a normal `Value`, JSON field or CLI argument, and its `Debug` form is always
redacted. Duplicate or malformed coordinates refuse rather than obeying a
hidden precedence rule.

The closed hosted shape uses exact-pinned `serde = 1.0.228` with `derive`.
`serde_json` was already a production dependency and `serde`/`serde_derive`
were already present through the pinned YAML/OpenAPI tooling, so this adds no
package to the lock; both are maintained foundational crates under
MIT-or-Apache-2.0 with MSRV 1.56/1.61, below Rastok's toolchain. Derivation
removes hand-written field dispatch and reliably enforces unknown-field
refusal. No serde type crosses the public/core boundary. The exit is local:
replace the hosted parser or construct `DeploymentProfile` directly without
changing source, plan, Driver or core contracts.

All bindings resolve atomically before the VM or journal exists. A VM whose
plan contains an absent or type-incompatible boot binding refuses before its
first mutation, even when a library caller bypasses the hosted CLI. A Driver
factory that does not explicitly consume a selected boot reference refuses at
binding. The real and scripted HttpClient share one logical prepared request:
it seals the URL and logical `SecretRef`, never the bearer bytes. The real
Driver overlays the already-resolved bearer only inside its authorized
boundary and only transports it after the OUTBOUND open; the scripted Driver
observes the same descriptor without receiving physical material. Request
identity therefore remains stable across secret rotation. Replay instead
requires the immutable run header to match the exact environment-name/value
digest and logical secret reference/provider coordinate/version used by the
booted VM.

Personal-data admission uses the same pre-effect boundary. The VM folds every
`@personal` path into required ordinary, Article 9 and Article 10 regimes. A
versioned `PrivacyProfile` must attest a processing-activity reference,
non-empty purposes and legal bases, explicit recipients and transfer policy,
and the additional Article 9 condition or Article 10 authority when those
flags occur. A personal-data path entering an external Connector requires at
least one declared recipient class; the core never guesses jurisdiction from
a URL, so an empty transfer-safeguard list remains an explicit “no
international transfer” attestation. M1 permits only `run_scoped` retention in `MemJournal`; dropping
the run drops its complete in-memory history. The run header records the
profile reference and canonical digest before the first personal value. VM
and journal headers must agree at every public execution entry point. Rastok
validates completeness and provenance of those facts, not whether the
controller's legal judgement is substantively correct. A matching header is
not a storage capability: `Journal::privacy_retention` is closed by default,
and only the M1 in-memory journal advertises `RunScoped`.

Raw `@secret` payloads remain fail-closed: A4 resolves reference-origin
credentials, not clear secret fields arriving in envelopes. A later durable
journal must prove field externalization or erasure before it can accept clear
personal data. A4 moves no categorical package, semantic witness or existing
causal trace. The run header is a fixed control attestation, not another
causal record species.

**Rejected:** putting secret bytes in source, profile JSON, CLI arguments,
`Value`, prepared-request bytes or causal records; treating environment values
as an authorization fallback for a secret setting; first-registration or
first-source-wins binding; parsing YAML/TOML in the no-std core; claiming GDPR
certification from a privacy-profile digest; and opening every historically
documented boot site without a concrete consumer.

## D-2026-08-30-2 — Personal data is a policy class, not a sensitivity level

M1.8 extends the closed schema-policy registry with `@personal`, canonical
identity `rastok/personal@1` and append-only annotation wire tag 5. This is the
GDPR-facing class: the Regulation speaks about *personal data*, which is wider
and more precise than the security acronym PII. Personal-data status is
orthogonal to `@sensitive` confidentiality and to `@secret` handling. A value
can be personal without being confidential, or restricted without identifying
any natural person.

The v1 source arguments are three optional booleans:
`special`, for Article 9 special-category data; `criminal`, for Article 10
criminal-offence data; and `identifier`, for a value used to identify or index
the data subject. No argument means ordinary personal data. The legal flags
accumulate under derivation. Exact copy and projection preserve `identifier`;
an unregistered calculation clears that role while retaining personal-data
taint. An alternative remains an identifier only when every alternative is
one. Personal data is redacted before presentation by default, independently
of its sensitivity level.

Purpose, legal basis, recipients, international-transfer safeguards and
retention are deployment facts, not type facts. A versioned privacy profile
attests them at boot and the run header records only its stable reference and
digest. The core validates presence and structural consistency; it cannot
certify that a controller's legal judgement is true. At the A3.1 checkpoint,
before A4 landed that boundary, every executable plan carrying `@personal`
refused before journal or state mutation. The M1 hosted runner may now
process personal data only with its run-scoped in-memory journal. A future
durable journal must prove an erasure/externalization mechanism before it may
accept clear personal fields; append-only causality is not permission for
unbounded retention.

This is an additive policy-registry move, not a categorical package move.
Every pre-existing annotation tag and ordering stays byte-identical, v15
remains the format, and unannotated as well as already-annotated witnesses and
causal traces do not move. Programs using `@personal` gain semantic identity
under the existing `rastok.metering.r1` policy extension.

These controls support privacy by design, data minimisation, storage
limitation, integrity/confidentiality and accountability. They do not make a
deployment legally compliant without the controller's processing record and
organisational measures. The legal vocabulary follows Regulation (EU)
2016/679 Articles 4, 5, 9, 10, 17, 25, 30 and 32.

**Rejected:** encoding PII as a fourth `@sensitive` level; using `@secret` as
the personal-data marker; putting purpose or legal basis in reusable source
types; allowing a durable journal to retain clear personal data merely because
the causal record is immutable; and claiming GDPR certification from schema
annotations alone.

## D-2026-08-30-1 — M1.8 separates data policy, accounting truth and disposable export

M1.8 replaces the old untyped Connector `cost` map with four typed schema
annotations beside the resolved logical type: `@sensitive`, `@secret`,
`@measure` and `@cost`. They never wrap or extend `DataType` and never alter
ordinary conformance or assignability. A canonical `SchemaPath ->
AnnotationSet` side-table carries their resolved meaning. Source annotations
that affect handling or accounting enter semantic identity; annotation order
does not. A schema with an empty table pays no runtime annotation walk.

Annotation arguments are closed plan constants. The initial path vocabulary
distinguishes the root, struct field, array element, map key/value, structural-
union branch and tagged-union case. Unknown annotations, duplicate kinds,
illegal targets, dynamic arguments and transported claims that cannot be
re-derived refuse by exact path. `@sensitive` propagates conservatively through
derived data. `@secret` is stronger and is not a secret origin: `secret(path)`
mints a `SecretRef` from attested boot bindings, while the annotation governs
handling. M1 admits only late resolution from a reference into a declared
secret-compatible Driver setting. A raw incoming secret-classified field
refuses before journaling until field-level externalization has its own gate.

The journal remains the accounting authority. Exact typed
`RecordObservation` values sit beside causal records but outside their seal;
the same fold produces `CostReport` with exporters disabled. Logs and hosted
OpenTelemetry are bounded never-block projections: saturation drops only the
projection and increments a drop counter. Redaction happens from the resolved
annotation table before formatting. High-cardinality flow identity belongs to
spans, logs and reports, not metric labels by default.

Semantic naming is standards-first. When an OpenTelemetry Semantic Convention
matches exactly, Rastok uses that canonical name directly. The `rastok.*`
namespace exists only for uncovered Rastok concepts; no Rastok alias shadows a
standard name. The initial snapshot is OpenTelemetry Semantic Conventions
v1.44.0 (`e10a930844c6951757a43b849d364f7d056ac32b`) plus the separate GenAI
repository at `67dff024110be5bd9f318006e733f4078e0f4c97`. GenAI's development
status is carried as registry metadata, so adopting a later definition is an
explicit registry migration rather than a silent change of meaning.

Monetary values are exact. `Money` is the canonical resolved struct shape
`{ amount: decimal(38,18), currency: text }`, not another logical type family.
A decimal cost annotation fixes an ISO currency in its constant arguments; a
Money-shaped value carries the validated currency itself. Provider-reported
and pricebook-calculated bases remain distinct, missing prices remain
`unpriced`, and reports never add currencies or apply an implicit FX rate.
The pinned ISO 4217 Maintenance Agency List One snapshot fetched on 2026-08-30
has SHA-256
`838dfb991648cf36df939edd5fe3811737962b75a32252847d239cedd1e291c9`;
adopting a later current-code list is an explicit registry migration.
Human deployment configuration is parsed only in the hosted crate (JSON is
already an exact-pinned production dependency there); the no-std core receives
typed `DeploymentProfile`/`BootBindings` values and parses no JSON, TOML or
YAML. `serde_json` remains dev-only in `rastok-core`; `serde_yaml_ng` remains
hosted OpenAPI input only.

Categorical package v15 carries the annotation side-table and registry
references. A2 lands an independent decoder and Registry-free validation of
the adjacent logical type, canonical path, target, measure carrier, currency
authority, ordering and local declaration claims. A3 keeps raw declarations
separate and independently derives the effective resolved-port table through
canonical operations before requiring exact equality with the transported
claim. The `DataType` registry and tags remain unchanged. Only annotated programs gain the
`rastok.metering.r1` semantic extension; concrete secret values, deployment
profiles, price profiles and exporter settings do not become graph identity.
Costs, metrics, host facts and export loss remain outside causal identity as
already required by `identity.md`. At the A3 checkpoint sensitivity and
accounting policy executed while clear secret-classified data retained the
named A4 refusal. D-2026-08-30-3 now opens only the reference-origin boundary;
raw secret payloads remain rejected. Transport never degrades into inert
decoration.

The A2 categorical move changes the version byte and checksum of every
package, but no unannotated body or semantic identity. Echo/spike remains 988 B
with v15 checksum `73ea33a7b0a9a4f25a109748b4a012c885d04cdf5aab91cc765052195394847a`;
JSONL remains 1,032 B with
`3805f1eb650f74d705eaaa83ef7af48f5df85ebd6c559d130414be813008bd26`;
gauge remains 323 B with
`a7e7e5743273b5a31ba3c3c616bde8fd260750acce753c7c750ea3186a45adcb`.
Their semantic witnesses stay `e50026ee…`, `036f7ea5…` and `0d3ad43d…`, and
all existing causal traces are byte-identical. The exact first annotated
semantic witness under `rastok.metering.r1` is
`f984f003953161e8fa93025787bd92a44fd18a8615aee1a0fbe4b9f1ba22defe`.

A3 adds no categorical tag, semantic extension or causal-record field. Exact
copy, projection and construction preserve policy; calculations and guards
retain the strongest security taint but do not infer measure or cost meaning;
alternative producers with incompatible accounting meaning refuse. Raw source
rows remain declarations while the resolved-port table carries the effective
derived verdict. Validation repeats the complete derivation without a
`ContractCatalog`, and execution never repairs it. Annotated immutable values
remain lexical paths in runtime trees so this dependency stays provable after
decode; unannotated constants keep their literal substitution and all
unannotated package bodies, semantic witnesses and causal traces stay exact.
Presentation redacts classified fields before formatting, and runtime faults
name value categories rather than payload contents. A plan with no declaration
takes an operation-tree zero-walk path. Opaque guests remain refused until
their boundary can apply the same conservative taint rule.

Budget enforcement is deliberately outside M1.8. The draft aspect would need
dynamic deployment policy, pre-effect reservation, plan-shaped typed refusal
and distributed accounting semantics; metering and OpenTelemetry cannot
authorize future spend. No `Budget` spelling, aspect tag, `BudgetRefused` case
or journal key is reserved now. FX, billing, FOCUS/OpenCost conformance,
sampled calibration, arbitrary user dimensions, raw secret envelopes and hot
secret rotation likewise remain named later work. Hosted OpenTelemetry export
is calendar-optional; annotations, secrets, structured logs and replayable
`CostReport` are not.

**Rejected:** annotations as `DataType` wrappers; arbitrary string maps or
user-defined annotation names; repeating units that a registered measure
already fixes; Rastok aliases for standard OTel names; OTel as accounting or
control authority; blocking an execution thread on export; redacting with a
post-format regex; treating unknown usage or price as zero; float money;
implicit FX; putting live budget amounts in source; moving YAML or another
human configuration parser into the core; serializing placeholder empty
annotation tables before A2; and reserving Budget identities merely
to preserve a design draft.

## D-2026-08-28-8 — Loop closes on public composition evidence, not a wider theorem claim

M1.7 L7 closes the first executable NodeAspect without broadening its proof
language. An ordinary nested Graph is a passive composition seam: lowering
removes that boundary and leaves one finite, scoped DAG. The Registry-free
Loop control walk therefore follows every edge and executable Node contained
by the Loop owner, including descendants of ordinary nested Graphs. It does
not treat a nested Graph name as another recursion or require a synthetic
summary. A nested Graph carrying its own `Loop` aspect is a different temporal
owner and remains fail-closed, as do direct spatial terminal edges to the Loop
boundary, explicit Loop `stop`, spatial Worker recovery and every theorem
fragment outside D-2026-08-28-7.

The public Lab closes the composition surface from source rather than through
private VM state: independent concurrent runs in one singleton instance;
independent keyed retries; an ordinary nested Graph in the body; a bounded
Process timeout followed by Loop delay; zero-or-more Process `out` frames
which cannot choose progress; exact exhaustion; and a finite instance
deadline which kills a stale wake before a clean successor generation runs.
The negative control proof still rejects a Connector stream as a progress
verdict. Thus a typed Call `exit`, not the number or arrival order of data
frames, remains the sole Connector fact from which this Loop gate may decide.

Expression binding is one rule at check and execution. The checker may resolve
the longest known dotted prefix such as `Loop.state.data`; the evaluator may
then traverse further members. The runtime consequently supplies both the
exact dotted aliases and the structural `Loop = { state = { data, meta } }`
root from the same immutable entry envelope. `Loop.state.data.field` cannot
compile and then fail because the root binding was absent.

L7 adds no source form, plan field, package tag, semantic extension or journal
fact. Categorical v14 containers, existing semantic hashes and every existing
causal trace remain byte-identical. The final report audits those three
identity registers separately and records the full all-feature, three-ISA,
independent-interop and Cortex-M33 evidence.

**Rejected:** treating every nested Graph as an opaque theorem boundary;
claiming nested Loop execution because ordinary nesting works; letting a
multi-frame Connector output select recurrence; adding a second Loop-state
binding semantics for long paths; removing named deferrals merely to make the
milestone look complete; and re-blessing identity artifacts for an execution-
coverage-only gate.

## D-2026-08-28-7 — The first Loop proof is a checked integer ranking certificate

M1.7 L6 admits the first deliberately small `Loop.measure` fragment. The
measure is a direct signed-integer path rooted at the iteration input
(`data` or `data.field…`). An invariant is either omitted or the independently
established literal `true`. Every recurrent path in this first gate is a
direct Graph `on in` handler whose guard proves the measure positive and whose
single `next` state writes that same scalar or struct field as
`measure - k`, for one positive integer literal `k`. The existing L3 control
proof remains the authority for disjoint/exhaustive progress and the existing
body contracts remain the authority for one finite traversal.

The hosted checker derives a certificate containing the checked measure,
optional invariant, and the sorted multiset of exact decrements. Sorting
makes evidence independent of source and handler-table order while preserving
duplicate paths. The small `no_std` core verifier derives the multiset again
from the transported canonical handlers and requires exact equality. A
certificate is evidence, never authority: unsupported activation, unsigned or
computed measure, unproved invariant, missing positivity, non-decrease,
multiple recurrence, or altered evidence refuses before a
`ValidatedExecutionPlan` exists. Nested summaries, Connector-result progress,
mutable-state claims, tagged-union case reasoning, lexicographic/custom
relations and a Rocq oracle remain named later theorem extensions.

At runtime an explicit positive `Loop.fuel` remains the operational cap. If a
verified measure stands alone, the manager evaluates it once on the root
iteration state and derives a safe internal application budget:
`max(measure, 0) + 1`. Strict descent while positive proves that this budget
contains every recurrent application plus the terminal application. The
budget then travels through the unchanged journal fuel fold; the runtime does
not re-prove, wrap, or repair a recurrence. With both declarations present,
the explicit fuel wins as the deliberate operational cap while the measure
remains a checked termination argument.

The transport is categorical plan-package v14. `LoopSpec` carries optional
fuel, optional delay and optional proof, and validation requires at least one
of fuel/proof. V13 is refused rather than interpreted as an absent proof. This
moves every package container and checksum, but no causal record: key 53 and
the run fold are unchanged. Existing fuel-only Loop source retains its exact
`rastok.aspects.r1` semantic bytes. A program that declares a proof adds the
domain-separated `rastok.loop-proof.r1` extension; the derived decrement list
does not enter source identity. Non-Loop semantic witnesses (echo/JSONL/gauge)
and every existing trace remain byte-identical. Exact v14 package sizes and
checksums are recorded in the identity checkpoint: echo/spike 988 B
(`afcc22ac…c0ebc`), JSONL 1,032 B (`1ca982c4…e54a06`), and gauge 323 B
(`cf8e9ac4…399bc5`).

**Rejected:** executing an unchecked measure as a runtime assertion; trusting
the compiler's witness list after decode; using an unsigned wraparound order;
accepting opaque Connector progress because it usually converges; making fuel
mandatory after a proof; treating proof-derived fuel as source identity; and
changing Loop causal facts merely because their admission proof became
stronger.

## D-2026-08-28-6 — A delayed Loop re-enters through one dated obligation

M1.7 L5 replaces only the scheduling edge of L4. A committed `next` still
advances the same journal fold, spends exactly one unit of fuel and mints the
same `l:next` envelope. With `Loop.delay`, that transaction additionally arms
timer kind 5 using the re-entry envelope id as the obligation reference. The
due instant is a persisted scheduler annotation, deliberately omitted from
canonical transaction key 53 and from semantic identity. Recovery restores
the same due instant; a date already reached becomes runnable immediately.

Selection and admission are two journal transitions. `Next` owns the selected
state, fuel decrement and optional dated obligation; `Admit` (extend-only
key-53 tag 5) is the sole `l:next` crossing into the next body application.
At `Admit`, the Worker selects every spatial destination, proves bounded
mailbox room, and commits those route facts atomically before delivery. A
duplicate admission therefore cannot execute the body twice, and an immediate
recurrence uses exactly the same crossing without a timer.

Fixed delay always uses its positive `after`. Exponential continuation number
`k >= 1` uses `min(ceiling, first * 2^(k-1))`; checked multiplication saturates
directly at the ceiling before any signed overflow is possible. The delay is
added to the VM's current logical instant. It changes neither fuel, a
Connector's independently armed timeout, nor an instance lifetime deadline.
Equal dates retain scheduler arm order, so production-shaped execution and
Lab `advance` share one deterministic causal order.

The dated payload carries the exact `InstanceRef`, run, ordinal and re-entry
id, but treats them as claims to re-check against the journal fold. A duplicate
or superseded event is therefore inert. If Closing wins before `Admit`, its
generation is no longer open and the event dies; it cannot target the
successor generation. Work whose admission already committed may drain as
admitted work, but the closing drain deadline discards any still-runnable Loop
work in that scope. Delayed work does not postpone an `idle`, `fixed` or
`tumbling` deadline: expiry enters the ordinary bounded Closing protocol.

The first spatial-body gate follows direct child emissions into their ordinary
Graph-owned handlers. The child transaction commits before the handler becomes
runnable, and the Loop fold accepts the decision only when its causal envelope
descends from the current iteration state. A bounded outbound Call retains its
own timeout: a timeout at `t` may select `next`, after which a two-second Loop
delay is due at `t + 2s`; the two budgets never share an arm or a deadline.
Direct Graph-boundary terminal producers remain fail-closed for the later
general verdict gate.

The in-memory journal retains the due annotation beside its typed Loop fact
so a fresh VM can re-arm a direct-body run without regaining fuel. A durable
journal may use another physical sidecar, provided its semantic query returns
the same annotation and its canonical causal projection remains the documented
key 53 array. Spatial recovery remains fail-closed until the general instance,
Connector-obligation and private-state recovery fold can rebuild its child
owners; L5 does not counterfeit that larger Worker-recovery claim.

**Rejected:** sleeping a Worker thread; a second Loop-only timer queue;
decrementing fuel when the timer fires; deriving delay from host wall time;
letting a wakeup migrate to a successor generation; treating an armed delay as
an extension of instance lifetime; selecting routes before the delay and
holding their mailbox capacity; pretending Loop-only replay reconstructs an
active Connector owner; and sealing scheduler due time into causal identity.

## D-2026-08-28-5 — `now()` observes one explicit transaction instant

Rastok admits `now()` as a context function in firing handlers. It returns a
`timestamp`: the VM's logical instant at the start of that transaction. The
instant is fixed for the whole firing, including guard selection, state
updates, emissions and scripted Connector handling. Evaluation remains pure:
the expression engine compiles `now()` to the reserved context binding
`now` and never reads an operating-system clock. Bare `now`, arguments to
`now()`, user bindings named `now`, and use outside a transaction handler are
refused.

Hosted execution samples civil time once at boot, then maps monotonic elapsed
time onto the VM timeline. Lab and embedded callers own the same explicit
timeline through `Vm::boot_at` and `Vm::advance`; the default origin is the
Unix epoch. A microcontroller without an RTC therefore still has useful
relative time from a hardware timer, without pretending to know the current
civil date. Internal deadlines, lifetimes, joins and Loop delays consume the
generic dated-event queue directly. No implicit clock port, clock envelope or
`meta.sys.created_at` field is introduced.

When a checked handler can observe `now()`, its TRANSACTION records seal the
fixed signed nanosecond instant under extend-only canonical key 54. The key is
omitted otherwise, preserving every time-insensitive causal record. This is a
deliberate exception to the rule that scheduler arming and due dates are
derivable annotations: an instant that can affect expression control flow is
an input fact and must replay as one. `Intent.logical_time` carries the same
instant to a scripted Driver but does not enter OUTBOUND call identity. Plan
validation derives guard/body time demand once from the canonical expression
trees; this derived cache is not transported and a time-insensitive firing
does no demand-discovery walk on its hot path.

The former deferred `Clock` Connector is renamed `Timer`. A `Timer` is an
ordinary inbound Event source for programs that need one-shot or periodic
timestamp envelopes; it is not the runtime's time authority. `Calendar`
remains the civil-schedule Event source. Neither is required merely to
evaluate `now()` or to run a timeout, lifetime, join eviction or Loop delay.

No package-format bump occurs: categorical v13 already represents the
canonical reserved path used by the context function. Existing semantic
hashes, packages and traces remain byte-identical unless a program newly uses
`now()`.

**Rejected:** evaluator-side `SystemTime::now`; one implicit clock input on
every Node; a readable envelope creation timestamp; forcing internal deadlines
through timer envelopes; keeping the ambiguous name `Clock` for both
the runtime timeline and an Event Connector; and recording every transaction's
time even when no expression can observe it.

## D-2026-08-28-4 — Loop fuel is an ordinary journal fold

M1.7 L4 executes the immediate, persistent root-Graph subset of `Loop`. One
accepted external root reserves the Loop owner generation and derives one
`LoopRunRef` from `(instance_address, generation, root_id)`. Distinct roots in
the same instance therefore retain independent state, ordinal and fuel. The
INBOUND route uses the reserved internal edge name `@loop.in`; it records the
selected owner generation before the run opens and is not a spatial back-edge.

One optional closed fact travels in ordinary TRANSACTION key 53. `Open` fixes
the run, owner generation, root, positive fuel and iteration-zero envelope.
`Next` advances the zero-based ordinal exactly once, decrements remaining fuel
exactly once and carries the manager-produced state envelope whose parent is
the selected `next` emission. `Close` records ordinary terminal output or
`stop`; `Expired` records the final selected `next` plus the exact
`LoopExpired { last_state }` envelope. Both are terminal. The journal refuses
duplicate open, missing/skipped/duplicated ordinal, wrong parent, premature
exhaustion and every transition after a terminal fact.

Domain `0x13` derives the placement-neutral run reference. Envelope domain
`0x05` derives immediate re-entry from `(run_ref, next_iteration,
selected_next_id)`. The last permitted body application that selects `next`
creates `Expired`, not `Next`: no `l:next` envelope exists beyond the fuel
boundary. A fresh Worker reconstructs every open immediate run by folding key
53 facts, rebuilds only disposable instance/scheduler state, and resumes from
the recorded envelope and remaining fuel. There is no mutable Loop ledger or
RAM attempt counter beside the journal. An implementation may cache the
replay-derived fold for indexed lookup; the cache is valid only when a fresh
fold of key 53 produces the same projection.

This gate deliberately keeps `Loop.delay`, spatial child bodies, nested Loop
owners, bare `stop` and finite owner lifetimes fail-closed. Their dated,
Closing and composition semantics land in L5/L7; L4 does not guess them. The
new domains and key occur only in newly executable Loop histories. Existing
semantic hashes, categorical v13 packages and causal traces remain
byte-identical.

**Rejected:** representing recurrence as an edge; deriving a run from only an
instance; decrementing fuel before an uncommitted traversal; committing
`l:next` and then translating it to expiry; keeping the live ordinal only in
the VM; and calling a cache replay when a fresh Worker cannot resume it.

## D-2026-08-28-3 — Loop proves finite progress, not one privileged result port

M1.7 L3 supersedes the `next XOR out` clauses of D-2026-08-28-1 and the
four-port wording of D-2026-08-28-2. `out` is a convention, not Loop control.
The fixed contract has exactly one Graph input, typed `in`, aspect-owned `next: In`, and
manager-produced `expired: LoopExpired<In>`; every other Graph output keeps
its domain name and is terminal for the current run.

One completed body application produces exactly one **progress verdict**.
Continuation is exactly one `next` with no ordinary output and no `stop`.
Termination is no `next` plus at least one ordinary output and/or bare `stop`;
several ordinary outputs may commit atomically as one terminal transaction.
Silence, repeated `next`, a user emission on `expired`, and a continuation
mixed with termination refuse before execution. Bare `stop` is deliberately
stronger than returning from one run: it closes the Loop owner instance and
therefore all runs scoped to that instance.

`Loop.fuel = N` proves at most N body applications even when every application
continues: the final proposed `next` becomes `LoopExpired`, so no theorem may
claim eventual `out`. A future standalone `Loop.measure` instead proves that
strict descent eventually reaches *some* explicit terminal verdict. In both
cases, each body application must itself finish under finite pure work,
Connector Call timeouts and guest budgets. Terminal spelling carries no
special authority.

**Rejected:** requiring a port named `out`; encoding every business outcome
inside one tagged union merely to satisfy Loop; treating arbitrary silence as
a return; allowing an auxiliary ordinary output beside `next` without an
explicit port-role distinction; and weakening `stop` into a per-run escape
that contradicts the existing instance lifecycle.

## D-2026-08-28-2 — V13 carries one closed Loop contract without executing it early

M1.7 L2 allocates categorical plan-package v13 for the first executable
`node::Aspect` transport. A Graph carrying `Loop` lowers one closed
`AspectSpec::Loop`: its positive plan-constant `fuel`, optional fixed or
exponential plan-constant `delay`, exact `in`/`next`/`out`/`expired` boundary,
and ordinary Graph-owned handlers. `Loop.invariant` and `Loop.measure` remain
theorem-tier refusals until L6; no partial or unknown aspect form travels.

The root Graph keeps one port authority: `ExecutionPlan::boundary`. An owner
row therefore carries an empty port table at the root, the complete boundary
only for a nested Graph, and no ports for a Node owner. This prevents two
transported claims from drifting while still making a nested Loop independently
decodable. Owner rows also carry a canonical closed aspect list. Handler
ownership becomes an explicit tagged choice between an executable Node slot
and a Graph owner slot; a Graph activation may name one direct child's typed
output. These are extensions of the ordinary handler and port mechanisms, not
a Loop-only executable AST.

After decode, the Registry-free gate re-derives the carrier and every
aspect-contributed port, requires each plan-constant expression to be closed
after lexical `val` substitution, re-evaluates its transported value, checks
positive delay/fuel constraints, canonical owner/aspect/handler order, direct
child ownership, expression types, and the prohibition on spatial `next`
wiring. Unknown, incomplete, misplaced, or forged claims refuse before the
validated-plan type is returned.

L2 deliberately does not execute the aspect. Until L3 proves one total
`next XOR out` decision and L4 installs the journal fold, every public mutating
VM door refuses a validated plan containing an aspect before it admits an
envelope, advances virtual time, or writes a journal record. `Vm::boot`
remains an allocation-only constructor and is not falsely described as an
execution gate.

Every package container moves categorically from v12 to v13 because the owner
and handler grammars widened; v12 is refused rather than guessed. Unchanged
programs retain their semantic hash because the added owner rows contain only
derived empty lists and the existing root boundary stays authoritative. A
new Loop program seals its checked contract and Graph handlers under the
`rastok.aspects.r1` semantic extension. No causal record moves at L2 because
no Loop plan executes yet; the connector-free gauge remains the negative
scope witness.

**Rejected:** mirroring the root boundary in its owner row; inferring an
aspect from port names or attributes after decode; carrying theorem syntax as
an unchecked expression; lowering a recognized aspect and silently running
it as a plain Graph; and changing `Vm::boot` solely to move a fail-closed gate
away from the first mutating operation.

## D-2026-08-28-1 — Loop is finite temporal recursion over one Graph boundary

M1.7 makes `Loop` the first executable `node::Aspect`, with one carrier and
one protocol: a Graph receives iteration state on `in`, emits either the next
state on aspect-owned `next` or its terminal result on `out`, and publishes
`LoopExpired { last_state }` on aspect-owned `expired` when its finite fuel is
spent. The checker proves exactly one `next XOR out` decision for every
reachable completed traversal. Other outputs may be emitted atomically but do
not decide. Multiple business terminal cases use a tagged union on `out`.

`next` is temporal control, not a spatial edge. One root envelope derives one
`LoopRunRef`; each run owns its fuel, zero-based iteration ordinal and pending
wakeup even when several runs share one keyed instance. A recurrence remains
in the same instance generation. Its `l:next` fact is parented by the selected
`next` emission and records the proposed state, next ordinal, remaining fuel
and optional due date. Recovery folds those journal facts; RAM never owns the
attempt authority.

The live surface is deliberately small and qualified:

- `Loop.fuel = N`, a positive plan constant bounding body applications;
- optional `Loop.delay`, either `{ kind = fixed; after = D }` or
  `{ kind = exponential; first = D; ceiling = M }`;
- theorem-tier `Loop.invariant` and `Loop.measure`;
- reserved, rejected `Loop.jitter`.

Absence of delay means immediate `l:next`. Exponential re-entry `k >= 1` uses
`min(ceiling, first * 2^(k-1))` with safe saturation. Delay never extends
fuel, Connector timeout or instance lifetime. The former unpublished
`Loop.variant`, `Loop.backoff`, `initial/source/value/until/limit`,
`max_steps` and `decreases` forms are removed with migration diagnostics;
they are not aliases.

Fuel has the exact recursion `run(0,s)=expired(last_state=s)` and
`run(N+1,s)=body(s)` followed by either terminal `out` or `run(N,s')` for
`next(s')`. Therefore a `next` proposed by the final permitted application
does not create `l:next`; the proposed state becomes `expired.last_state`.
Fuel bounds applications, not the duration of one application. Calls inside
the body require finite positive timeout, guest work its finite budget, and
open Event/Session waits refuse in v1.

At least one termination witness is mandatory. Fuel is the first executable
structural witness. A future checked `measure` may stand alone, or accompany
fuel as proof plus operational cap. The verifier must establish invariant
entry/preservation, body termination, total `next XOR out`, and strict
well-founded decrease on every `next` path. It treats Connector results as
fresh symbolic typed values and may use only explicit path guards. Hosted
proof search produces a certificate checked by a small deterministic
`no_std` core verifier; unsupported proof fragments fail closed. The term
`measure` follows Rocq/Coq well-founded recursion vocabulary. `variant` stays
a data-type word.

`Loop.state` is the sole read-only aspect binding and denotes the complete
iteration-entry envelope (`.data` and `.meta`). There are no Loop lifecycle
hooks or hidden writable state. Instance lifetime is orthogonal: Closing
invalidates pending Loop wakeups, while the termination theorem is conditional
on the owner remaining open.

**Superseded:** D-2026-08-27-4's tentative Graph/Operator/Connector
compatibility; D-2026-08-12-1 examples using `Loop.backoff`; and every older
Loop clause that described `variant/until`, runtime decrease checks,
Connector-local retry, `Loop.exit`, or an iteration as a new instance
generation.

**Rejected:** three carrier-specific continuation protocols; direct Loop on a
Connector merely because it has an outbound Driver role; an unqualified
`fuel`/`delay`; `until` as a second base-case mechanism; runtime assertions
presented as termination proofs; a separate Loop ledger; wall-clock or
placement facts in `LoopRunRef`; and an unbounded admitted Loop.

## D-2026-08-27-4 — Every Node has one kind, one type, and zero or more aspects

The three classifications are orthogonal and use one vocabulary in the
specification and Rust API: `node::Kind`, `node::Type`, and `node::Aspect`.

- `Kind` is structural and is fixed by the declaration keyword: `Graph`,
  `Operator`, or `Connector`.
- `Type` is the one effective implementation/contract within that kind.
  `Graph` and `Operator` are implicit defaults; `App` is the unique root
  Graph type; every Connector names one exact versioned contract type.
- `Aspect` is an additional orchestration protocol. A Node may carry none or
  several, in canonical order, but an aspect never replaces its type and
  cannot execute before its checker, plan, runtime, Lab, and identity gates
  have landed. The closed initial vocabulary is `Loop` and `Saga`.

The source slot is `: Type? (+ Aspect)*`. When an explicit type is present it
must be first. Omission is canonical for ordinary Graphs and Operators:
`operator x {}` and `operator x : Operator {}` are the same program. A
Connector type is mandatory. `Saga` is Graph-only. Under D-2026-08-28-1,
M1.7 admits `Loop` on Graphs only; direct Operator/Connector carriers are
later gates. The current executable build classifies the spelling and refuses
it by the named aspect gate until its categorical transport lands. A
duplicate, unknown, misordered, or incompatible symbol is
`E-NODE-ANNOTATION`.

`Reducer`, `Aggregator`, `Window`, and `Joiner` are user-guide names for
ordinary Operator parameterizations, not NodeTypes: Node-local `var`, optional
`instance.key`, optional `lifetime`, and an ordinary or bounded-join handler
fully determine them. `Dispatch` is the corresponding keyed-Graph pattern.
`Router` is likewise a multi-output conditional-emission pattern; the name
does not become executable syntax until the general conditional routing
surface can express it without a privileged runtime family. A genuinely
different pure implementation such as a future `Jaq` or `Wasm` engine may be
an Operator NodeType.

The common addressing attribute is singular: `instance = { key = <expr> }`.
It describes the policy of the one declared Node; runtime collections and
ordinary prose still use the plural “instances”, while the contextual value
remains `instance.key`. The unpublished plural source spelling is a one-voice
`E-INSTANCE` migration refusal. The already-published identity extension
domain `rastok.instances.r1` is historical binary vocabulary and does not
move merely to mirror source spelling.

This preparatory change adds no aspect to an executable plan and therefore
does not bump the v12 plan package, semantic hashes, or causal records. The
current type of every executable plan entry is derivable from existing facts:
root Graph owner → `App`, other Graph owner → `Graph`, Operator entry →
`Operator`, Connector spec → `Connector(id)`. The first executable aspect
will pay for one categorical transport extension and its identity moves in
M1.7; the package will never carry a second contradictory kind/type claim.

**Rejected:** `trait | role | aspect` as three annotation categories (`App`
is a type, and “trait” collides with Rust implementation vocabulary);
interchangeable annotations where any symbol can act as the principal type;
stdlib aliases that make ordinary state/lifetime combinations look like
different runtimes; implicit execution of a recognized but unimplemented
aspect; renaming the historical identity domain for cosmetic consistency.

## D-2026-08-27-3 — Hosted capability selection is static registry assembly

The shipped standalone image has one explicit assembly function and one
ordinary `DriverRegistry`. `Process` is always present, `File` is added only
when deployment supplies a physical root, and the hosted protocol stacks are
selected by the additive Cargo capabilities `hosted-http12`, `hosted-http3`,
`hosted-websocket`, or their `hosted-network` union. The full preset binds
Process, File, HttpClient, HttpServer, WebSocketClient, and WebSocketServer in
one VM. A reduced image that encounters an omitted contract refuses during
global binding, before VM construction, journal mutation, listener activity,
or an external effect.

The plan continues to carry only the stable semantic Connector id, roles, and
exchange. A Cargo feature, crate, TLS backend, library path, Host, Worker, and
physical protocol version never enter plan or causal identity. The registry's
data-only manifest is the exact projection of the statically linked image; it
does not load code and is not an authorization to execute. Future native,
Wasm, or out-of-process plugin hosts must produce the same ordinary factories
and pass the same pre-VM binder. M1.6 deliberately freezes no Rust/native ABI,
serialized plugin manifest, artifact locator, trust model, or hot-reload
protocol.

This is physical assembly only. Existing semantic hashes, package bytes, and
causal traces do not move. The connector-free gauge remains the negative scope
witness.

## D-2026-08-27-2 — One causal Session serves WebSocket over HTTP/1–3

H6 binds `rastok/websocket-client@1` and `rastok/websocket-server@1` to the
generic Session exchange without inventing a call per message. HTTP/1.1
Upgrade, RFC 8441 HTTP/2 Extended CONNECT, and RFC 9220 HTTP/3 Extended CONNECT
are interchangeable physical openings of the same contract. Physical
capability selection and any version fallback occur before a peer can observe
CONNECT/Upgrade bytes; after that visibility boundary, failure is one portable
terminal observation and never a retry through another version.

The accepting INBOUND fact opens the placement-neutral `SessionRef` before a
server reports handshake success. An outbound opening and every graph-to-peer
message have an OUTBOUND Session intent before protocol bytes. The Driver
reports opening confirmation and local write settlement, while the VM alone
commits the sole terminal Connector transaction under `l:exit`. That
transaction carries key 50, emits exactly one final `exit`, waits until every
open write is settled or abandoned, projects the same reference as
`meta.ext.session_id`, and makes later messages, writes, and exits non-causal
observations. A preparation refusal before the opening intent is an ordinary
typed completion of the causing delivery and has no `session_id`: no logical
Session existed to name.

The WebSocket Driver owns fragmentation, ping/pong, UTF-8 and the close
handshake. One complete text or binary message is one envelope. Codec mismatch,
oversize input, handshake refusal, protocol failure, transport loss, timeout,
and ambiguous settlement remain distinct portable `WebSocketExit` cases.
Inbound body/event capacity, active sessions, per-session command lanes,
physical message size, and the VM's pending Session-write table are finite.
When the latter reaches the Worker mailbox bound, the mailbox remains claimed
until a settlement frees capacity; moving a write from the mailbox into the
durable fold can therefore never create an unbounded side queue.

Session routing is a real execution boundary for DAG validation. A server
message and a later graph reply are different temporal facts even though the
same both-role Driver owns them; the validated spatial graph does not add a
fictional in-to-out edge through the Connector. A server may own many
concurrent logical sessions and a client at most one per Node instance.
Correlation is always by `SessionRef`, never a socket, stream, QUIC connection
id, or arrival order.

This gate allocates the extend-only lifecycle spelling `l:exit` (tag 10) and
timer kind 9 for the outbound opening deadline. The existing v12 package,
semantic recipes, and all prior causal facts remain byte-identical: only new
Session executions can contain these new forms. The connector-free gauge and
all existing package/trace goldens remain unchanged.

## D-2026-08-27-1 — One bounded HttpServer Call serves HTTP/1–3

H5 closes `rastok/http-server@1` without introducing a version-specific
contract. Hyper HTTP/1.1 and HTTP/2 streams and quiche HTTP/3 streams enter
the same retained `InboundCallRef` protocol. Request-count, stream-count,
header, body, event and response capacities are finite. The Driver acquires
body capacity before copying bytes, the VM commits the root before `ack`
returns 202, and `respond` writes only an already-journaled graph envelope.
The standalone runner polls inbound Drivers independently of stdin.

`respond` requires a positive instance `timeout`. The VM arms timer kind 8
in the accepting INBOUND fact; response and timeout are two protocol actions
competing for one retained request and one `l:reply`. Whichever action the
Driver starts must report the sole terminal observation. A graph response
that loses the race is consumed as an orphan and never writes bytes. Direct
lineage selects the opening root. A path crossing a root-mixing join must
carry an explicit `correlate = <direct data or meta path>` whose static result
is text; selection occurs before the producing route commits.

Malformed framing, failed authentication and undecodable/oversized bodies
remain pre-admission HTTP refusals. They cannot be `HttpServerExit` cases
because no causal Call exists. H5 therefore removes `MalformedRequest` and
`AuthFailed`; overload remains `Overloaded` only when an admitted obligation
cannot be served, while listener/body pressure before admission remains an
HTTP response with no root. The representative HttpServer semantic identity
moves from `63a257e2…` to `b83fcf2d…` because the resolved `exit` union is
identity material. Other network witnesses, Process/JSONL/echo and the
connector-free gauge `0d3ad43d…` do not move. Existing causal traces and plan
format v12 do not move; timer kind 8 occurs only on new respond-mode roots.

HTTP/3 keeps TLS mandatory, 0-RTT disabled and the same in-memory identity as
the other bindings. Wire version, connection ids and stream ids remain
physical observations. Public-network probes are not release gates; H7 owns
the pinned independent interoperability ledger.

## D-2026-08-26-7 — A graph response is the return half of its inbound Call

The generic inbound-Call protocol now accepts one already-journaled graph
response without turning it into a second outbound Call. An inbound-only Call
Connector has two independent boundary halves: an external request creates a
fresh root at its output, while a graph delivery to its input terminates at
the retained external request. There is no implicit input-to-output execution
edge through the Connector. The strict DAG is therefore checked over actual
execution dependencies; an edge ending at this response sink does not create
a spatial cycle back to a later external request.

The response-producing transaction selects and seals the destination before
commit. Only after that commit may `RuntimeBindings` offer the retained
envelope to `InboundCallConnectorDriver::start_response`. `Pending` means no
protocol byte was written and leaves the same delivery behind the Connector's
ordinary claimed mailbox; `Started` transfers the bounded write attempt to
the Driver, which must later report one typed terminal observation. The VM
then commits the existing `l:reply` transaction with the opening root as its
causal parent and the response delivery's real cursor in `consumed`. Fixed
`ack` continues to consume zero cursors. Neither form writes an OUTBOUND
record or invents a synthetic delivery.

This checkpoint executes direct lineage only: the response `origin_id` must
name the open request. A response arriving after closure, a second response,
or an unmatched lineage is consumed by an ordinary empty transaction, so the
discard is journaled and can never create protocol bytes or a second `exit`.
Explicit `correlate` remains part of the deferred HttpServer target contract;
it must select the opening source instance during pre-commit route
preparation, not repair a route after commit. Bounded request deadlines,
protocol cancellation, the live respond Driver and HTTP/3 remain later H5
work. Consequently the public HttpServer sheet and real factory still admit
only `mode = ack`.

This is H5.2's generic response-return checkpoint. It allocates no plan field,
format tag, semantic recipe or causal key. It only permits a new combination
of existing transaction keys 33, 39 and 52 on new response facts. Every
existing package, semantic hash, causal trace and golden remains
byte-identical.

## D-2026-08-26-6 — A live ack listener holds bytes and acknowledgement until admission

The first physical HttpServer checkpoint binds the already-published
`rastok/http-server@1` sheet to Hyper's HTTP/1.1 and HTTP/2 server engine. It
does not create version-specific contracts and it does not put an async or
Hyper type in the Driver boundary. `HttpServerFactory` is a statically
selectable inbound factory; binding resolves every listener, route, codec,
body bound, response header, bearer digest and optional Rustls identity before
the hosted runtime thread starts.

The listener owns finite connection, request-stream, event-count and shared
byte capacities. HTTP/2 advertises the same finite concurrent-stream bound.
Before copying each request-body chunk, the hosted pump acquires its byte
permits; the permits travel with the retained proposal and are released only
when the runtime accepts or rejects it. A full causal mailbox therefore leaves
the proposal and its transport response pending under the same finite
capacities. It never creates an unbounded side queue or a detached task per
terminal observation. The Driver drains proposals and terminal observations
from the common bounded lane independently, so a later pending request cannot
strand an earlier `exit`.

`mode = ack` returns 202 only after `admit_inbound` committed the root, exact
source instance and `InboundCallRef`. The local protocol terminal then enters
the same bounded event lane as `Completed { status: 202 }`; the VM alone
commits the close-once `l:reply`. Cleartext HTTP/1.1 and prior-knowledge HTTP/2
exercise this path through real loopback peers. Optional in-plan certificate
and key bytes select the already-vetted Rustls server assembly. Authentication
uses a precomputed digest and constant-time digest equality. The configured
bearer and library errors do not enter terminal payloads; an admitted peer
header remains subject to the existing lossless `http_headers` contract.

Server binding is the fail-fast provisioning boundary: it may reserve every
passive listener of the family before returning, but it may not acknowledge a
request, mint a causal fact or perform an outbound effect. A request that
arrives before the Worker begins pumping remains held under the same finite
connection/request/byte capacities and observes no acceptance.

HTTP framing/authentication/body refusals that occur before a typed proposal
has been admitted return their portable HTTP status without minting a root or
an `InboundCallRef`. Consequently they cannot truthfully become an
`l:reply`. This exposes an H2 contract debt rather than papering it over:
`MalformedRequest`, `AuthFailed`, and pre-admission `Overloaded` are currently
published in `HttpServerExit` although no admitted Call can produce them. The
final H5 contract pass must remove or reclassify those cases; it must not
invent a fake Call merely to make them reachable.

This is H5.1, not H5 closure. HTTP/3 listener integration, `respond`, bounded
pre-admission/request deadlines, disconnect/write races, standalone-runner
polling and independent Caddy/NGINX/curl/nghttp2/ngtcp2 evidence remain H5
gates. No plan field, package byte, semantic hash, existing causal trace or
root recipe changes here. New wire requests use keys 27, 28 and 52 already
allocated by D-2026-08-26-4/-5.

## D-2026-08-26-5 — An inbound Call opens at its source and closes once

The generic inbound proposal protocol now carries the whole Call lifecycle,
without pretending that an inbound request is an outbound invocation. Before
acceptance, the runtime selects the exact source Connector instance and every
downstream route in one pure preparation. The accepting INBOUND commit opens
the Call on that instance; only then does the Driver receive
`Accepted(InboundCallRef)`. `InboundCallRef` is the committed root-envelope id
behind a direction-specific newtype, not a second digest and not a socket or
stream id.

INBOUND key 28 seals `[inbound_call_ref, source_instance_ref]`. The source
instance is a real route reservation even when no downstream edge exists, so
its lazy `l:start`, state, Closing and replay generation use the same authority
as every ordinary destination. Pending Calls are bounded per source instance
by the Worker mailbox capacity; admission that cannot reserve all required
capacity commits nothing and sends no acceptance callback.

An inbound Call Driver has one categorical terminal-observation view. The
registry proves that the reporting family admitted the reference, the
resolved port gate validates its `Completion`, and the VM commits an ordinary
Connector TRANSACTION with trigger `l:reply`, cause equal to the root request,
no consumed mailbox cursor, and key 52 naming the Call. It may emit ordinary
typed outputs, but exactly one conventional `exit` must be last. The journal
refuses an unmatched `l:reply`, a missing/repeated/non-final `exit`, a crossed
source instance, and any second terminal transaction. A late Driver fact is a
non-causal observation. No OUTBOUND record and no synthetic input delivery are
created.

Keys 28 and 52 are extend-only and omitted from every pre-H5 record. No plan
package, semantic hash, existing causal trace or root-id recipe moves in this
checkpoint. A fake retained HttpServer request proves commit-before-accept,
source-instance routing through an Operator, and one typed terminal exit.
D-2026-08-26-7 later extends the same `l:reply` form from fixed
acknowledgement's zero cursors to exactly one already-journaled response
cursor; it does not reinterpret the opening fact or manufacture an input.
This is the generic H5 Call seam, not the live listener claim: protocol accept
loops, fixed `ack` bytes, graph-driven `respond`, cancellation/timeout,
HTTP/1.1–3 flow control and independent interop remain in H5.

## D-2026-08-26-4 — A Connector-origin root names its source endpoint

H5 closes an ambiguity left by the generic inbound proposal seam. A retained
Driver proposal's `InboundEnvelopeProvenance` names the exact Connector Node
whose output introduces the root. The bound family proves ownership of that
path, and the validated plan proves that the selected port is an output of an
inbound-capable Connector. There is no second source-path claim to reconcile.
Every check happens before sequence reservation or journal mutation.

The distinction between an App-boundary injection and a Connector-origin root
is causal, not a runtime convention. INBOUND key 27 carries the source
Connector path as an array of lexical path segments; its absence means the App
boundary. Key 24 remains the selected port in both cases. Existing boundary
records remain byte-identical, while new live Event/Call/Session sources can
replay from `connector.out` without guessing from provenance text. The root id
recipe remains `(connector_path, replica, inbound_seq)` and does not move.

This was the first H5 checkpoint, not a claim that an HTTP listener already
executes. D-2026-08-26-5 subsequently lands the generic accepted-request and
close-once terminal seam without lowering it into a second outbound Call.
HTTP/1.1 connections, HTTP/2 streams and HTTP/3 streams must all implement
that one protocol before their wire Drivers are admitted.

Hosted interoperability evidence is split by purpose. The H5/H6 blocking
gates will use pinned independent local peers (Caddy and NGINX for server
behavior; curl/nghttp2/ngtcp2 for client behavior; Autobahn plus an independent
WebSocket implementation for H6). Scheduled public canaries will exercise
Cloudflare/Google endpoints but will never be the deterministic release
oracle. No test may silently depend on the public Internet.

## D-2026-08-26-3 — Type the HTTP schema boundary and resolve OpenAPI losslessly

HTTP content types are no longer anonymous `text`. Rastok adds the open logical
scalar `media_type`, carried by canonical UTF-8 text but validated as one
concrete Internet media type. Type, subtype, suffix and parameter names are
case-normalized, parameters are ordered, duplicate names and wildcards refuse,
and `media_type(text)` is the one expression constructor. The standard-library
catalogue offers common completion candidates without closing the domain:
vendor and newly registered media types remain legal. `Accept` remains an
ordinary custom header until a distinct range-and-quality type exists; a
concrete `media_type` is deliberately not stretched to mean a media range.

`meta.ext.content_type` now has logical type `media_type?`. Whole-payload
`asset` and `base64!` arguments are canonicalized at check, direct metadata
writes are contextually narrowed, and the live HttpClient projects a valid
response Content-Type through the same parser. Invalid peer spellings remain
available byte-for-byte in `meta.ext.http_headers` but do not fabricate the
typed projection.

Request configuration separates two concerns. `headers` is a sparse typed
struct; its first standard field is `content_type: media_type`. The table may
grow with independently typed well-known fields. `custom_headers` is the open
extension namespace, `map<text,array<text>>`: source may use conventional HTTP
casing, names normalize to lowercase before the request digest, and a
case-insensitive collision or attempt to override Driver-owned fields refuses.
Both maps still become the pre-open lossless `map<text,array<bytes>>`
descriptor shared by HTTP/1.1, HTTP/2 and HTTP/3. There is no parallel
`mime_type` spelling.

Project tooling gains `rastok --resolve-open-api <project-dir> [--check]`.
It parses every Rastok source with the host parser, discovers static
HttpClient URL/method and HttpServer path declarations, finds the unique
OpenAPI 3.0/3.1 document describing each endpoint, and writes the deterministic
`openapi.generated.rkg`. It never edits an authored file. All selected
components pass through the one `DataType` algebra; recursion, optional-field
absence, unsupported constraints, ambiguous unions and unrepresentable source
names refuse rather than weakening to `variant`. `--check` makes stale output
a build failure. A generated name that collides with an authored type refuses
instead of shadowing or adopting it. The generated file is ordinary Rastok
source and is reparsed by the real grammar in the tool's conformance test.

The RFC parser is exact-pinned `mediatype 0.23.0` (MIT, no_std,
allocation-only, default features disabled, no transitive runtime dependency and no
`unsafe` in that crate). Upstream declares no MSRV, so Rastok makes no
second-hand promise: the workspace toolchain and three no_std ISA builds are
the executable compatibility gate. The crate is small and single-purpose;
Rastok owns canonicalization and the public error surface, so replacing an
unmaintained or incompatible release would not change `DataType`.

Project documents use exact-pinned `serde_yaml_ng 0.10.0` (MIT, declared MSRV
1.64) and `serde_json 1.0.151` (MIT OR Apache-2.0, declared MSRV 1.71) only in
the std compiler/tool; neither reaches the no_std core or runtime plan.
`serde_json` uses its ordinary std feature. `serde_yaml_ng` has no optional
feature surface and brings `indexmap`, `itoa`, `ryu`, `serde` and the small
`unsafe-libyaml` parser into hosted tooling only. None is exposed through a
Rastok API. The maintenance posture is deliberately conservative: exact pins,
the lockfile, deterministic fixtures and the private reader boundary are the
gate, and the exit path is to replace that reader while retaining the same
internal OpenAPI tree and refusal surface. These dependencies remove
RFC/schema-parser risk that would otherwise sit in this repository; license
inventory and three-ISA no_std gates keep their cost and boundary visible.

This is categorical plan-package **v12**. V11 is refused rather than guessed;
the append-only allocations are type-tree tag 18 for `media_type`, expression
function id 6 for `media_type(…)`, semantic-HIR DataType tag 26 and semantic
function id 6. Every package golden moves because the container version is
sealed. Semantic hashes move only for programs whose checked trees now carry
the new logical type or typed-header contract; causal traces move only when
such a semantic hash is present in their envelope identity. These moves are
enumerated before re-blessing, with the non-network gauge fixture retained as
the scope witness.

**Landed identity checkpoint.** The source witness
`base64!("image/png") { AAEC }` moves `fc8cddb6… → ee1dab9e…` because its
checked metadata write is now `media_type`, not anonymous text. A typed-header
HttpClient has the newly allocated identity `c66bd9da…`; it has no pre-H4.1
equivalent because the old open `headers` bag is deliberately rejected rather
than reinterpreted. The unchanged no-new-field network witnesses retain
HttpClient `481662aa…`, HttpServer `63a257e2…`, WebSocketClient `d4f92e15…`,
and WebSocketServer `d36bcbf4…`. Compiler probe `2933618d…`, JSONL
`036f7ea5…`, echo/spike `e50026ee…`, and connector-free gauge `0d3ad43d…`
all keep their semantic hashes. Exactly `echo.rkp` (974 B), `jsonl.rkp`
(1 018 B), `gauge.rkp` (313 B), and the spike's binary echo copy move from
v11 to v12; their bodies and sizes are unchanged, with only version and
checksum differing. No causal trace is re-blessed.

The Connector code layout follows contracts rather than vague sharing names:
flat `http_client`, `http_server`, `websocket_client` and `websocket_server`
modules own family assembly; `websocket_protocol` names the actual shared
protocol state, and private `network` owns only sockets, TLS, pumps and codecs.
`common`/`shared` modules were rejected because they do not state an invariant.

## D-2026-08-26-2 — One live HttpClient contract over HTTP/1.1, HTTP/2 and HTTP/3

M1.6 H4 binds the already-published `rastok/http-client@1` contract to the
private hosted engine selected by H3. It does not introduce one Connector per
HTTP version. The ordered `versions` row selects physical attempts among
HTTP/3, HTTP/2 and HTTP/1.1 implementations compiled into the runtime. A
version may be skipped or another attempted only before request bytes can have
become externally visible. Once visibility is possible, every failure is the
terminal portable fact for that call; Rastok never retries an ambiguous write.
Redirects and status-based retries likewise remain explicit graph policy.

The family binder constructs one canonical request descriptor before the
OUTBOUND open: method, resolved absolute URL, lossless ordered headers, exact
codec body, bearer token and idempotency key. The journal seals its exact
digest and the live Driver decodes those bytes after open; it does not rebuild
the request from mutable envelope or environment state. `max_body` bounds both
directions. An oversized request yields `BodyTooLarge { limit }` before open in
both the live and scripted Drivers. A response is accumulated only up to the
same bound, with checked reservation before every copy. Header parsing and
projection are separately bounded by the physical Driver configuration.

Every syntactically and protocol-valid response, including non-2xx status,
emits one `out` followed by exactly one `Completed { status }` on `exit`.
`out` carries `meta.ext.http_status`, `http_version`, lossless
`http_headers: map<text,array<bytes>>`, and canonical typed `content_type`
when present (superseded from anonymous UTF-8 text by D-2026-08-26-3).
The selected wire version is observation only: equal bodies decode to equal
payload values and never move semantic identity. Invalid body content maps to
`DecodeFailed`; DNS, connection, verified TLS, protocol and body-limit facts
map to their closed `HttpClientExit` cases. Scheduler timeout and bounded
Closing remain the sole authorities for `TimeoutExpired` and
`OutcomeUnknown`; OS and library strings never enter causal payloads.

HTTP/1.1 uses origin-form targets plus a Driver-owned `Host` field. HTTP/2
uses one pooled sender per origin and TLS-material identity, so concurrent
streams may complete in reverse order without crossing calls. HTTP/3 uses
quiche over verified TLS and bounded QUIC/H3 flow control; H4 opens one bounded
connection per call, while the H3 adoption spike remains the multiplexed
backend proof. HTTP/3 requires HTTPS. H1/H2 use Rustls, H3 uses BoringSSL, and
both parse the same optional CA/client-identity bytes before DNS or socket
creation. User `host`, `content-length`, hop-by-hop and duplicate synthesized
authorization/idempotency fields refuse as `ProtocolFailed` rather than
silently overriding Driver protocol state.

Outbound local saturation is pacing, not an external failure. The generic
Call start handshake is therefore `CallStart::Started | Pending`. `Pending`
starts no effect, leaves the already-journaled call open and causes the
dispatcher to offer the same intent again after capacity returns. Only
`Started` enters the dispatcher's exactly-once local start set and the
family-correlation table. This is the outbound twin of two-phase inbound
admission; it does not add a queue or causal record.

The H3 pump's work permit remains charged while the network task owns the
request, and is released before the completed response enters the same finite
event budget. Work, event and control lanes stay bounded; cancellation has an
independent priority lane. Per-call body storage is bounded by `max_body` and
the number of concurrent tasks by the physical `max_in_flight` limit. A
stopped engine is `ConnectFailed`; saturation is always `Pending`.

H4 also makes the H2 lossless header schema representable in envelope
metadata. A metadata leaf now admits the closed carrier
`scalar | array<scalar> | map<text,array<scalar>>`; namespace paths remain
flat and prefix-free. The new map reuses canonical value tags 18 (map), 19
(array), 8 (text) and the existing scalar tags. No old value changes bytes.
This narrowly supersedes D-2026-08-10-1's scalar/array-only leaf sentence.

The direct `http 1.5.0` dependency exposes the request/header/URI vocabulary
already present transitively through Hyper and quiche; it adds no locked
package. Its MIT/Apache-2.0 license and Rust 1.57 floor are below the selected
hosted stack. It stays optional behind `hosted-http12` or `hosted-http3`, no
third-party type crosses the private network module, and removing or replacing
a backend preserves the semantic sheet and Driver boundary.

The hosted cost was measured rather than inferred. On 2026-08-26, with Rust
1.97.1 on an Apple M5 Pro (`arm64`, macOS 26.6.2), the standard release binary
was 4,835,168 bytes before stripping and 3,951,728 bytes (3.769 MiB) after
`strip -x`. The all-feature image containing the H1/H2/H3 hosted stack was
10,328,592 bytes before stripping and 8,608,448 bytes (8.210 MiB) afterwards:
a marginal 4,656,720 bytes (4.441 MiB, +117.8%). Its dynamic dependencies were
only macOS `libc++`, `libiconv` and `libSystem`; BoringSSL is part of the image.
This is a physical-build observation, never semantic identity or a portable
size promise.

Evidence includes real loopback H1 status/header/body streaming, one verified
TLS H2 connection with two reversed concurrent streams, real-UDP verified H3,
fallback before request visibility, refusal to fall back afterwards, one
graph-value matrix across all wire versions, finite-bound tests, and the
connector-neutral Call conformance battery. The battery proves Completed,
ConnectFailed, TlsFailed, ProtocolFailed, BodyTooLarge, DecodeFailed and
TimeoutExpired. `DnsFailed` remains explicitly unprovable without replacing
the public host resolver; `OutcomeUnknown` is proved by the generic Closing
model rather than fabricated by a Driver test.

H4 changes no plan/package format, semantic identity recipe or causal record.
Existing package, hash and trace goldens remain byte-identical. New response
metadata is a world observation on newly executed network calls, not a change
to any pre-H4 program. Rejected alternatives are version-specific Connector
ids, fallback after a possibly visible request, unbounded queues or response
bodies, library errors in exit payloads, making the VM async, and reporting
local queue pressure as `ConnectFailed`.

## D-2026-08-26-1 — One bounded hosted pump, Hyper/Rustls for H1-H2, quiche for H3

M1.6 H3 adopts the physical networking substrate without attaching a live
Driver to any semantic contract yet. The VM remains synchronous. Every hosted
network client and server uses the same private pump interface in
`crates/rastok/src/connectors/network`. Each bound engine owns one dedicated
current-thread Tokio runtime, bounded work/control/event channels, and one
shared byte semaphore. Work and control have distinct finite lanes and control
is selected first, so a full request queue cannot starve cancellation. Work
bytes remain charged while queued and while the network runtime owns the work;
event bytes remain charged until the synchronous side drains them. Dropping
the synchronous handle signals shutdown, closes both directions and joins the
thread. Protocol flow-control windows may be tighter, never wider than the
declared physical byte budget. All-feature evidence drives real Hyper H1 and
H2 exchanges and constructs the quiche H3 configuration from inside this same
pump interface; the standalone H3 spikes then prove real UDP progress.

The selected static hosted assembly is:

| Capability | Pinned implementation | Selected features | License / declared MSRV |
|---|---|---|---|
| Reactor/readiness | `tokio 1.53.1` | `io-util, macros, net, rt, sync, time`; no multi-thread scheduler | MIT / 1.71 |
| HTTP/1.1 and HTTP/2 | `hyper 1.11.0`, `hyper-util 0.1.20`, `http-body-util 0.1.5` | client, server, H1, H2 and Tokio I/O only | MIT / 1.63–1.64 |
| H1-H2 TLS | `rustls 0.23.43`, `tokio-rustls 0.26.4` | Ring, std, TLS 1.2 and 1.3 | Apache-2.0/ISC/MIT / 1.71 |
| HTTP/3 and QUIC | `quiche 0.29.3`, `boring 4.22.0` | `boringssl-boring-crate`; defaults off | BSD-2-Clause + Apache-2.0 / 1.88 + 1.80 |
| WebSocket message machine | `tungstenite 0.30.0`, `tokio-tungstenite 0.30.0` | handshake only; TLS supplied outside it | MIT/Apache-2.0 / 1.85 |
| Trust material | `webpki-root-certs 1.0.9` | complete DER Mozilla root corpus | CDLA-Permissive-2.0 / 1.70 |
| Glue | `bytes 1.12.1`, `futures-util 0.3.34`, `rustls-pemfile 2.2.0` | minimum used features | permissive licenses |

The Cargo features are deliberately physical and orthogonal:
`hosted-http12`, `hosted-http3`, `hosted-websocket`, and their union
`hosted-network`. A build may omit BoringSSL, Hyper or Tungstenite without
changing a Connector id, sheet, plan or semantic hash. The feature closures
on the adoption host contain respectively 68, 71, 55 and 128 unique Cargo
tree entries including Rastok and build dependencies. BoringSSL requires a C
and C++ toolchain, CMake, bindgen and clang; Rustls Ring also compiles its
small native/assembly support. No selected dependency enters `rastok-core` or
an embedded target.

**The deliberate TLS duplication is accepted.** H1-H2 use Rustls with the
Ring provider; H3 uses the BoringSSL context required by the selected quiche
feature. Both consume the same exact compiled Mozilla certificate corpus.
Absent `tls_ca` selects that corpus. Present `tls_ca` replaces it with a PEM
certificate bundle or one DER certificate; it never disables verification.
`tls_certificate` accepts a PEM chain or one DER leaf.
`tls_private_key` accepts an unencrypted PEM PKCS#8/PKCS#1/SEC1 key or an
auto-detected unencrypted DER key. The pair is parsed and checked in memory
before a socket exists. Rustls advertises `h2,http/1.1`; quiche advertises
`h3`. Rustls early data is false/zero and quiche's enabling API is never
called, preserving H2's categorical `zero_rtt = disabled`. The staged quiche
configuration caps connection and per-stream receive credit at the engine byte
budget, caps peer bidirectional streams at the work-queue capacity, reserves at
least three peer unidirectional streams for HTTP/3 control and QPACK, and uses
a finite 30-second transport idle timeout. These are physical safeguards, not
new semantic settings; H4-H5 must retain finite bounds when live Drivers land.

**quiche wins the H3 comparison for this gate.** Its direct synchronous QUIC
state machine maps cleanly onto the bounded pump, exposes client and server
from one version, and is deployed by Cloudflare. The real-UDP spike proves
verified TLS, H3 ALPN, two concurrent streams, reverse completion and
reclamation. A fresh release build took 21.58 seconds and produced a
2,114,216-byte stripped evidence binary with 64 locked packages.
The competing `h3 0.0.8` + `h3-quinn 0.0.10` + `quinn 0.11.11` spike is
smaller in native-tooling complexity, shares Rustls, built in 12.55 seconds,
and produced a 2,729,488-byte stripped binary with 85 locked packages. It
also interoperated as a verified public-root client with Cloudflare's quiche
endpoint. It is not selected because the [upstream h3 project](https://github.com/hyperium/h3)
still labels its API highly experimental and its current close/reclamation
behavior has [active edge cases](https://github.com/hyperium/h3/issues/338).
That is an integration-risk judgment, not a throughput
claim; the isolated comparison remains the substitution path if maturity or
the dual-TLS cost changes. No third-party type is part of Rastok's public
Driver contract.

With the same `strip -x` measurement, the currently shipped CLI grows from
3,881,384 to 3,998,528 bytes under all three staged features (+117,144 bytes),
because H4-H6 have not yet linked live factories; the quiche spike is the
honest lower-bound evidence for an exercised H3 image. H4-H7 must re-measure
once actual Drivers are reachable. Removal is mechanical: disable a physical
feature or replace its private backend while retaining the same pump and
semantic sheets.

Tungstenite is proved with finite read/write/message/frame limits over a
synthetic duplex byte stream and through Hyper's real HTTP/1.1 Upgrade path.
That does **not** prove RFC 8441 or RFC 9220. HTTP/2 and HTTP/3 WebSocket remain
H6 gates until their Extended CONNECT stream adapters, settings negotiation,
flow control and close races pass independently. Likewise H3 adopts engines,
not live HttpClient/HttpServer behavior: journal-before-effect, portable
outcomes and negotiation/fallback remain H4-H5.

H3 changes physical dependencies and build outputs only. It changes no
contract, plan/package format, semantic identity recipe or causal record; no
existing hash, package golden or trace is authorized to move.

Rejected alternatives are making the VM async, one runtime per protocol,
unbounded async queues, Tokio types in `rastok-core`, h3/Quinn as the present
H3 backend, native platform roots whose contents vary by host, one TLS
provider forced across all protocols, enabling 0-RTT, or treating a successful
Tungstenite HTTP/1.1 proof as evidence for Extended CONNECT.

## D-2026-08-25-2 — Four network sheets precede every network engine

H2 publishes the semantic network boundary before adopting a socket, TLS,
HTTP, QUIC or WebSocket implementation. A validated sheet determines the
stable Connector id, Driver roles, exchange lifecycle, complete effective
port table, setting phase, body/message codec, bounded intake and portable
terminal union. A hosted Driver may implement those facts; it may not fill in
one that the plan omitted or reinterpret one according to its library.

| Id | Roles | Exchange | Ports |
|---|---|---|---|
| `rastok/http-client@1` | outbound | call | `in: T`, `out: T`, `exit: HttpClientExit` |
| `rastok/http-server@1` | inbound | call | `in: T`, `out: T`, `exit: HttpServerExit` |
| `rastok/websocket-client@1` | both, one shared binding | session | `in: T`, `out: T`, `exit: WebSocketExit` |
| `rastok/websocket-server@1` | both, one shared binding | session | `in: T`, `out: T`, `exit: WebSocketExit` |

For all four, required `codec = json | text | bytes` selects
`T = variant | text | bytes`. One HTTP body or one complete WebSocket message
is one value: these codecs add no `lines`/`jsonl` delimiter. Network contracts
have no implicit `err`; response bodies, including non-2xx HTTP bodies, are
ordinary `out`, while transport/protocol/lifecycle facts are cases of the one
terminal `exit`. Non-portable library and OS detail remains observation.

`versions` is a non-empty, duplicate-free ordered subset of `http/1.1`,
`http/2`, and `http/3`; its default preference is
`["http/3", "http/2", "http/1.1"]`. The first mutually supported version may
be selected only before request/session bytes become externally visible. The
chosen version is projected as observation and does not change contract or
graph identity. `zero_rtt` is fixed to the sole value `disabled`; no Driver
may treat a different spelling as ordinary one-RTT traffic.

HTTP headers use one lossless portable shape:
`map<text,array<bytes>>`. Field names and their value arrays are non-empty;
names are lowercase ASCII HTTP tokens, repeated values retain wire order, and
pseudo-fields remain Driver protocol state. Standard `meta.ext` projections
are `content_type: media_type` (superseded from `text` by
D-2026-08-26-3),
`http_headers: map<text,array<bytes>>`, `http_method: text`,
`http_uri: text`, `http_version: text`, `http_status: int`,
`session_id: text`, `websocket_origin: text`, and
`websocket_subprotocol: text`. A Driver overwrites peer-provided session
identity and derives protocol observations from the admitted exchange.

TLS is verified, never opportunistically plaintext. Optional `tls_ca` is
trust-anchor material; `tls_certificate` and `tls_private_key` are byte
settings that must occur together. Absence selects deployment trust roots or
no client identity on a client; a server without an in-plan pair requires a
deployment-provided server identity for every TLS version it offers. It never
means disabled verification or an opportunistic plaintext downgrade.
`auth_bearer` is an explicit token setting. H3 records the accepted
certificate encodings and root/identity assembly with the selected TLS engine;
it cannot weaken these sheet rules.

The exact terminal unions are:

- `HttpClientExit = Completed { status: int } | DnsFailed | ConnectFailed |
  TlsFailed | ProtocolFailed | TimeoutExpired |
  BodyTooLarge { limit: uint } | DecodeFailed | OutcomeUnknown`;
- `HttpServerExit = Completed { status: int } | MalformedRequest |
  AuthFailed | RequestExpired | Overloaded | PeerDisconnected |
  OutcomeUnknown`;
- `WebSocketExit = Closed { code: int, reason: text } | DnsFailed |
  ConnectFailed | TlsFailed | HandshakeRejected { status: int } |
  ProtocolFailed | TransportLost | TimeoutExpired |
  MessageTooLarge { limit: uint } | DecodeFailed | OutcomeUnknown`.

Every usable HTTP response, regardless of status class, publishes its body on
`out` with `meta.ext.http_status`; `Completed.status` closes the same call.
Automatic status retries are not part of the Connector. HttpServer `ack` is
the executable H2 profile; `respond` remains a named H5 deferral because it
must retain and settle a bounded remote response obligation. The WebSocket
contracts expose complete messages only; fragmentation, control frames and
handshake mechanics remain Driver duties.

H2 supplies family-owned Lab bindings but no live network. The HttpClient Lab
twin seals the same method, URL, headers, body and idempotency material that
the real Driver will receive. Passive server/session bindings prove exact
catalogue-to-registry composition without pretending to implement a socket.
The Registry-free core independently rejects altered roles, exchanges,
attributes, phases and ports.

H2 changes neither categorical package format nor an existing identity
recipe. It makes formerly invalid network programs valid and extends the
standard `meta.ext` vocabulary, so only new programs can mention these facts.
All pre-H2 semantic hashes, v11 package bytes and causal traces must remain
unchanged. No product network dependency is added.

## D-2026-08-25-1 — HTTP versions are Driver bindings; WebSocket is a session

M1.6 opens the network boundary. Supporting HTTP/1.1, HTTP/2, HTTP/3 and
WebSocket must not turn four wire protocols into four execution models, nor
turn a long-lived duplex channel into a sequence of fictitious request/reply
calls.

**Driver roles and exchange shape answer different questions.** The two roles
from D-2026-08-22-1 remain exhaustive: `InboundConnectorDriver` brings facts
from the world and `OutboundConnectorDriver` performs journaled actions on the
world. A separate closed fact, `ConnectorExchange`, describes the lifecycle of
the interaction:

- `Event` admits one spontaneous external fact as an inbound root and has no
  per-event terminal obligation;
- `Call` admits one operation, may publish ordered outputs, and ends with
  exactly one typed call `exit`;
- `Session` opens once, carries zero or more messages in either direction,
  and ends with exactly one typed session `exit`.

This is cardinality, not another Driver role. The then-named `Clock` (renamed
`Timer` by D-2026-08-28-5) and `Calendar` use event semantics. `HttpClient`,
`HttpServer` in ack/respond mode, `Process(job)` and
`File` use call semantics.
`WebSocketClient` and `WebSocketServer` use session semantics and require both
Driver roles over one shared bound state. `Process(service)` will reuse the
same session shape when that deferred profile lands; the VM must never grow a
WebSocket-specific branch.
The executable pairings are closed: Event is exactly inbound; Call may be
inbound, outbound or both-role; Session is outbound or both-role because its
opening, write-settlement and terminal observations enter through the
exchange-specific outbound view. A future purely inbound session protocol
would need an explicit terminal proposal form rather than silently losing its
exit.

**WebSocket has two contracts, one message model.** The stable target ids are
`rastok/websocket-client@1` and `rastok/websocket-server@1`. Opening a remote
connection and accepting one have different attributes and failure surfaces,
so they are not profiles of one ambiguous node. Both expose ordinary `in`
(graph to socket), `out` (socket to graph), and `exit` (one session terminal)
ports. They do not acquire an implicit `err`: a protocol or transport failure
is a typed `exit` case, while non-causal library diagnostics belong to
observation. One complete WebSocket message becomes one envelope and one
envelope sent to `in` becomes one complete message. Fragmentation and
reassembly, ping/pong and the close handshake remain Driver protocol duties.

Each accepted or opened session receives an opaque logical `SessionRef`
derived from its journaled opening fact. It excludes file descriptors, socket
addresses, negotiated HTTP version, Host, Worker and library handles. The
reference is carried by causal routing and projected on session messages and
the terminal envelope as `meta.ext.session_id`; it is never a physical socket
id. A server reply on a join-free descendant path targets the session by
lineage. A root-mixing path, a stored response, or a spontaneous producer must
carry an explicit contract `correlate` value; no Driver guesses from arrival
order. A client Connector owns at most one live remote session per logical
Node instance in v1, while a server Connector may own many accepted sessions.
The Driver overwrites any peer-supplied spelling of `session_id` at ingress
and resolves a correlator only inside that Connector's live logical table. A
write targeting an unknown or already closed reference is an orphan write:
discarded and journaled for observation, never redirected to another session,
never retried forever, and never allowed to manufacture a second exit.

**Session writes are effects, not calls.** Every graph-to-socket message is
journaled before any bytes are offered to the network. Its write obligation is
closed by the session protocol without manufacturing an application `exit`
per message. A write failure terminates the session through its sole typed
`exit`. After a crash, a message whose external outcome is unknown is never
silently replayed: the session terminates as unknown and the graph may open a
new session and retry through an application acknowledgement or the `Loop`
aspect. Reconnection and application-message replay are graph policy, not
hidden Driver behavior.

**Inbound acceptance is two-phase.** A Driver proposes a request, accepted
session, or session message without minting an envelope. The runtime either
commits it and acknowledges acceptance to the Driver, leaves it pending under
backpressure, or rejects it with a contract result. An HTTP 202/response,
WebSocket upgrade success, or explicit application acknowledgement may leave
only after the corresponding causal fact commits. TCP acknowledgements and
HTTP/2-3 receive-window updates are transport flow control, not evidence that
Rastok accepted or processed a message; they may advance only within the
Driver's declared bounded intake. WebSocket defines no per-message
application acknowledgement — a graph that needs one sends an ordinary reply
message after commit.
H1 replaces the former pull-only `InboundConnectorDriver::next` API with a
retained proposal plus `Accepted | Pending | Rejected` decision, before a
network Driver ships.

**HTTP versions are physical bindings by default.** The semantic contracts
are `rastok/http-client@1` and `rastok/http-server@1`; there is no `Http2` or
`Http3` node. A compatible Driver may negotiate HTTP/1.1, HTTP/2 or HTTP/3,
and the negotiated version is an observation rather than semantic identity.
Fallback is allowed only before request bytes could have become externally
visible. Zero-RTT is disabled in v1; enabling replayable early data requires a
later idempotency decision. A future explicit contract requirement for a wire
capability may restrict negotiation, but M1.6 does not infer such a requirement
from the URL or from registration order.

WebSocket uses RFC 6455 framing over an HTTP/1.1 Upgrade, RFC 8441 Extended
CONNECT over HTTP/2, and RFC 9220 Extended CONNECT over HTTP/3. Applications
see the same session contract on all three paths. A peer or build that lacks a
required transport refuses or negotiates another compatible version before
admission; it never executes a nearby protocol silently.

**Network libraries stay behind hosted Drivers.** The isolated `quiche`
0.29.3 spike is a GO for HTTP/3 candidacy: it proves verified TLS, `h3` ALPN,
two correlated streams and a synchronous pump without changing the product
workspace. It does not supply RFC 6455 framing. The implementation target is
therefore a small hosted engine with an HTTP/1.1-2 backend, an HTTP/3 backend,
and a separately tested WebSocket codec. `quiche` is the presumptive HTTP/3
candidate and `tungstenite` the presumptive framing candidate; `h3` + Quinn
remains the comparison that can win if one Rustls/TLS stack or integration
cost outweighs the measured quiche advantages. Dependency versions, TLS
providers and an async reactor are adopted only after the product-shaped gate
records licenses, transitive cost, cancellation, flow control and exit
strategy.

Tokio, if selected for hosted socket readiness, is an implementation detail of
the std Connector crate. It does not enter `rastok-core`, the plan, contract
types or no-std builds. Embedded builds omit the network Drivers entirely and
continue to validate a plan while refusing unavailable contracts at binding.

**Simulation substitutes the network, not the protocol.** Lab factories use
the same contract ids, exchange shapes, session correlation, typed exits,
backpressure decisions and journal boundaries. They script peer messages and
transport outcomes after the same open/commit gates. Conformance has separate
profiles for calls and sessions; a successful request battery cannot
stand in for fragmentation, interleaving, close-once and post-exit silence.

The exchange shape and session reference require categorical plan and causal
transport additions. M1.6 allocates plan-package v11 and the causal additions
to its H1 gate, with pre-declared semantic/package/trace moves. No identity
moves occur in this decision-only checkpoint.

**H1 identity checkpoint (declared before re-blessing).** Adding the closed
exchange tag to canonical Connector HIR moves every semantic hash whose graph
contains a Connector: the echo/spike Process graph, the JSONL Process fixture,
and the compiler probe. The connector-free gauge hash is the negative witness
and must remain `0d3ad43d…`. The categorical v10→v11 container moves all three
package goldens (`echo.rkp`, `jsonl.rkp`, and `gauge.rkp`) plus the spike's
embedded echo copy; the gauge body remains semantically identical. The new
session causal fields are optional and omitted on existing Event/Call facts,
so none of the four existing trace goldens may move. Exact replacement hashes
and spike measurements are recorded only after the implementation passes its
gates.

**H1 landed checkpoint (2026-08-25).** The declared moves are exactly:
compiler probe `52406fcf… → 2933618d…`, JSONL
`9643616e… → 036f7ea5…`, and echo/spike
`190f0efc… → e50026ee…`; connector-free gauge remains
`0d3ad43d…`. The three package goldens and the embedded spike copy move to
v11, while all four causal trace goldens remain byte-identical. The final
Cortex-M33 run decodes the 974-byte package in 16 906 ticks, measures the
relay at 2 641 average / 2 551 minimum / 2 877 maximum, peaks at 129 660 heap
bytes, links 531 724 bytes of `.text + .rodata`, and prints `SPIKE_OK` with
the exact new hash. The first measurement crossed both tight resource bounds;
the correction removes a redundant whole-transaction clone and keeps the
rare inbound-session extension out of every Event/Call record's inline size,
without changing any canonical byte.

Rejected alternatives are:
one node per HTTP version, one generic `WebSocket` contract, one Call per
WebSocket message, forcing pure event sources into sessions, automatic
reconnect/replay, a third `DuplexDriver` role, network-library types in the
core, and an embedded HTTP dependency enabled by default.

## D-2026-08-22-1 — Contracts name capabilities; Drivers supply them by role

M1.5 opens the connector catalogue with a second real family (`File`). The
existing runtime could execute only a single `Process` implementation, whose
binder rejected every other callable contract. That shortcut must not become
the architecture: one validated plan may contain several connector families,
and every callable or source connector must be bound before the VM and journal
exist.

**Vocabulary and responsibilities are separate.** A **Connector** is the Node
declared by an RKG program. A **Contract** is its compiler-visible semantic
sheet: stable identity, roles, ports, types, attributes, phases and lifecycle
rules. A **Driver** is one runtime implementation of that contract. A
**Binding** is the verified association between a plan Connector and the
Driver instance that will serve it. `adapter` remains an ordinary English
description at external-format boundaries; it is not the runtime interface
name.

There are exactly two runtime roles, named relative to the Rastok boundary:

- an `InboundConnectorDriver` proposes external arrivals to the runtime;
  sequence minting, route preparation, mailbox admission and journal commit
  remain runtime authorities;
- an `OutboundConnectorDriver` starts already-journaled calls, reports their
  correlated completions and accepts cancellation requests. It never writes
  records or manufactures scheduler outcomes.

A contract may require either role or both. When it requires both, one
Connector factory creates one shared implementation state exposed through
both role interfaces; two unrelated instances may not silently divide its
protocol. This two-role model covers Process job/service, File
read/replace/watch, HTTP client/server, clocks, queues, databases and duplex
transports without a third connector kind.

**The plan names the contract, not the binary.** Every connector contract has
a stable, versioned identifier whose canonical spelling is
`<namespace>/<name>@<major>`, for example `rastok/process@1` and
`rastok/file@1`. The identifier is semantic and travels in the categorical
plan. A shared-library path, Rust crate version, feature flag, Host, Worker,
or concrete Driver version is physical deployment data and never enters the
plan, semantic hash or causal journal. Unknown identifiers, unavailable
Drivers, missing required roles and duplicate matching registrations all
refuse by typed diagnostics; selection is never first-registration-wins.

The compiler owns a **ContractCatalog** containing declarative sheets. The
runtime owns a **DriverRegistry** containing implementation factories. Their
keys meet only in the binder, which produces one `RuntimeBindings` table for
the validated plan before the first VM, journal record or external effect.
The core VM routes by the binding and role; it does not match on Process,
File or any future connector name. Registry-free plan validation continues to
verify every transported contract fact independently of either registry.

**Static composition is first-class; plugins remain compatible, not frozen.**
M1.5 supports explicit static registration, so embedded, minimal standalone
and full standalone builds may link different Driver sets. Cargo features may
be convenience presets, but semantic registration remains explicit and
testable. A future trusted `.so`/`.dll` loader or out-of-process plugin host
must register through the same DriverRegistry and publish a contract manifest
readable without executing plugin code. M1.5 deliberately does not freeze a
Rust dynamic ABI or a native plugin lifecycle; the stable boundary today is
the contract identifier plus the semantic Driver protocols.

**Simulation substitutes Drivers, never the machine.** The Lab builds mock
Inbound/Outbound Connector Drivers against the same contracts and bindings.
The production VM still mints envelopes, applies backpressure, journals
intent before effect, validates typed completions and closes calls. A mock may
change external outcomes but may not weaken a contract or bypass binding.

**`File` is the proof of the architecture, not a filesystem API.** The M1.5
contract addresses exactly one file per invocation. `read` and atomic
`replace` are the initial outbound modes. A path may name a file below
existing directories, but the connector does not list directories, recurse,
create parents or remove trees. The single-file `watch` spelling is reserved
by a named deferral until initial snapshot, replacement, deletion, overflow,
resynchronization and acknowledgement semantics are decided. Because watch is
an Inbound role while `rastok/file@1` is Outbound, lifting it also requires an
explicit contract-version or separate-contract decision; the deferred source
word promises no v1 port shape. Directory traversal, if ever admitted, is a
separate contract.

The first implementation checkpoint only adopts this vocabulary and module
boundary and therefore moves no semantic identity. The versioned identifier
transport is a categorical plan-format change allocated to M1.5's next gate;
it must use one explicit format bump and enumerate every semantic/package
golden move before re-blessing. This decision supersedes the `Ingress` and
`Partner` interface names and the single-Process binder as an architecture,
while preserving their already-proved prepare/commit, correlation,
completion, cancellation and conformance semantics.

**C1 identity checkpoint (2026-08-22).** A `ConnectorId` has exactly the
canonical spelling `<namespace>/<name>@<major>`: the namespace is one or more
dot-separated lowercase ASCII labels, the name is one lowercase ASCII label,
labels start with a letter and may contain digits or internal hyphens, and the
major is a positive canonical `u32` decimal without a leading zero. No alias
is normalized. A source symbol such as `Process` resolves through the
`ContractCatalog`; only its stable id (`rastok/process@1`) and required roles
enter canonical HIR and the plan. Two source symbols may not claim one id,
and a `DriverRegistry` rejects a duplicate physical implementation rather
than selecting by registration order. Binding distinguishes an absent id
from an implementation missing a required role.

Plan-package **v10** categorically replaces v9 by transporting the canonical
id and the closed role value (`inbound`, `outbound`, or both) in every
Connector row. Roles use new Connector-spec key 6; retired `error_set` key 5
remains unassigned forever. V9 is refused, not migrated. This is a deliberate identity
move for Process programs: compiler probe `29c69b70… → 52406fcf…`, JSONL
`a058f1aa… → 9643616e…`, and echo/spike `419a52a1… → 190f0efc…`.
The connector-free gauge remains `0d3ad43d…`, proving scope. All three
package goldens move to the v10 container; Process packages additionally
replace the source name with the id and add the role. Existing causal traces
remain byte-identical because neither contract identity nor roles are causal
record fields.

The former `IngressAuthority` type is named
`InboundEnvelopeProvenance`: **the provenance of inbound envelopes**. It is
the opaque, stable `(connector_path, replica)` root-stream fact shared by the
envelopes accepted from that logical Connector replica; the journal combines
it with `inbound_seq` to mint each root. It is neither a network address nor a
per-submission id. This vocabulary change preserves the root recipe and every
causal byte.

**C2 binding checkpoint (2026-08-22).** `RuntimeBindings::try_new` is the
single application binder. It groups active Connector Nodes by exact
`ConnectorId`, resolves each factory once, and requires the returned shared
object to expose exactly the roles its factory advertised. An inbound-capable
Node is active by role; an outbound Node is active when an edge can deliver to
it. An outbound-only pure source is therefore skipped rather than mistaken
for an unserved caller. Missing ids, insufficient advertised roles, a
factory refusal, and an advertised/actual role contradiction are distinct
pre-execution failures.

After binding, the VM knows only `OutboundConnectorBindings`: Node paths route
request preparation, starts, cancellations, scheduler-owned terminal facts,
and responses to the selected family. Request framing and the typed projection
of `TimeoutExpired`/`OutcomeUnknown` belong to that bound contract; the VM
never matches `Process` or another concrete Connector id. The completion gate
continues to judge every returned value solely from the validated resolved
port table. `ProcessFactory` is explicitly registered by the CLI and binding
finishes before either the VM or journal is constructed. A direct single
Driver still implements the aggregate interface automatically for focused
conformance tests; registry-backed Lab composition remains C5.

The mixed-family proof executes two different fake contract ids through one
VM, while a both-role proof reaches both interfaces on one shared bound
object. Binding failures start no Driver effect and create no journal. C2
moves no semantic hash, package byte, causal trace, or format version.

**C3 File-contract checkpoint (2026-08-22).** `rastok/file@1` is an
outbound, one-file contract with two executable profiles and one named
deferred profile. `read` accepts `in: variant`, emits exactly one complete
`out: bytes` on success, and terminates on `exit: FileExit`; `replace`
accepts `in: bytes`, emits no success payload, and terminates on the same
exit port. `watch` is known but deferred and its placeholder row is not an
executable role/port promise. The user must write `mode`; intent is never
inferred from payload or wiring. Both executable profiles have
predeclared `effect = idempotent` and `behavior = deterministic`.

`path: text` is required and may close as late as invocation. Its checked
expression travels in the existing v10 dynamic-attribute row. Immediately
before request preparation, the VM evaluates it against the admitted
envelope plus the immutable instance bundle, validates the result, and gives
the Driver only the resulting value map. A fault opens no outbound record.
Plan values have already been bound by the factory; instance and invocation
values are resolved into the same per-call map. A Driver never receives an
expression tree, consults the ContractCatalog, or injects a default. The exact
values used for request preparation ride with the in-memory intent so start
observes the same call that was sealed.

The logical path is UTF-8 text relative to an explicit physical root supplied
when the Driver is bound. That root, a crate path, mount, Host and sandbox
policy are deployment facts and never enter the semantic plan. C4's Driver
must reject empty or absolute paths, NUL, empty segments, `.` and `..`, and
native-invalid spellings as `InvalidPath`; it creates no parent directory.
It preserves the written Unicode, case and segment spelling and never calls a
semantic `canonicalize`. The root is an address base, not a promised security
sandbox: native symlink resolution is external-world behavior unless a
deployment supplies a stricter Driver.

`read` is all-or-nothing and bounded by `max_read`, default `"8MiB"` under
the existing portable 4-GiB byte-size grammar. A limit refusal publishes no
partial `out`. `replace` writes a temporary file in the target directory and
publishes it through one native atomic replace; a platform that cannot make
that guarantee returns `AtomicReplaceUnavailable` and must never fall back to
truncate/in-place writing. The contract promises atomic visibility, not
crash durability (`fsync`) or preservation of permissions, ownership, xattrs
or timestamps. Existing directory segments are required; only the final file
may be created.

`FileExit` has the exact portable cases `Completed`, `NotFound`, `NotAFile`,
`PermissionDenied`, `InvalidPath`, `ReadLimitExceeded { limit: uint }`,
`AtomicReplaceUnavailable`, `IoFailed`, `TimeoutExpired`, and
`OutcomeUnknown`. Paths, host messages and error numbers do not enter those
payloads: the intent already identifies the operation and path, while
unstable physical detail belongs to observation. Directory listing,
recursion, implicit parent creation and removal are absent sheet attributes;
the single-file watch protocol remains a named deferral.

C3 performs no filesystem observation and registers no File Driver. It adds
no format tag and moves no existing semantic hash, package or causal trace;
new File programs use the already allocated v10 connector and phased-setting
forms. C4 alone implements the external effect.

**C4 File-Driver checkpoint (2026-08-23).** `FileFactory` now binds
`rastok/file@1` to one explicit physical root without inspecting that root.
Pure request preparation validates the logical path and seals one versioned
operation descriptor: mode, exact UTF-8 logical path, and either `max_read` or
the exact replacement bytes. Those descriptor bytes — not a later
reconstruction from attributes — are the request-digest material and the sole
operation a worker executes after OUTBOUND is open.

The logical separator is `/`; `\` is rejected so one plan cannot reinterpret a
segment as a separator on another host. Native absolute forms are rejected in
addition to the C3 lexical refusals. Joining the path to the root and every
filesystem observation happen only in the post-open worker.

One detached worker per call keeps blocking filesystem I/O outside the
synchronous machine. Read checks the declared limit before retaining each
chunk and publishes either one complete raw-byte `out` followed by `Completed`,
or no `out` and one portable refusal. Replace opens a collision-safe
`create_new` staging file in the target directory, writes it completely, then
uses the host's native same-directory atomic rename. The target is never used
as staging, and there is no truncate fallback. On platforms where replacing
an existing target cannot be guaranteed atomically, the result is
`AtomicReplaceUnavailable`.

Cancellation is cooperative and best effort. If observed before read
completion or the rename commit point, the worker publishes nothing; the
scheduler owns the terminal timeout/unknown fact. If the external operation
wins the race, its completion may arrive late and the generic close-once
dispatcher makes it graph-invisible while retaining it as an observation.
Dropping a bound Driver requests cancellation of all remaining workers.
`Dispatcher::resume` marks existing open calls as already started: only the
future recovery fold may authorize a recall, so replay alone never observes or
mutates a file.

Host messages, paths and error numbers stay outside causal values. The Driver
maps only the portable FileExit distinctions and collapses every remaining
host failure to `IoFailed`. C4 changes no plan format, semantic hash, package,
or causal trace; File request digests begin to exist only for newly executed
File calls.

**C5 conformance/composition checkpoint (2026-08-23).** The conformance
battery now accepts a `DriverRegistry`, performs the same global
`RuntimeBindings` pass as the application, and derives the terminal union from
the validated resolved port table. Its happy case and interleaving oracle are
profile data rather than Process assumptions. The real Process and File
factories both pass that one battery; every portable terminal case is either
provoked or carries an explicit host-independent reason why it cannot be.

The Lab also composes mock Drivers through exact contract ids and role binding.
For each supported family, its mock factory reuses the real Driver's pure
plan-bound request preparation and scheduler-terminal projection; only the
post-open external effect is scripted. The production VM still opens and
closes every OUTBOUND record and applies the resolved completion gate. A mixed
Process/File scenario proves both calls on the journal, while the same graph
compiled without the Lab has the identical semantic hash and validated plan.
A malformed File mock output refuses at check as `E-PORT-TYPE`.

Static build composition is explicit and tested in three forms:
Process-only, File-only and their combined registry. A reduced registry does
not claim a family it did not register, and the VM contains no Process/File
dispatch. No semantic hash, package byte, causal trace or format version moves
at C5.

The promised cost comparison is recorded without inventing elapsed time. Over
Rust sources and tests, the one-time C0–C2 architecture span
`147d8c3..475e7c7` changed 3,335 inserted and 1,061 deleted lines; the
File-specific C3–C4 span `475e7c7..1c6e11d` changed 2,482 inserted and 94
deleted lines. These are reviewable change-size facts, not person-hours. The
repository captured no trustworthy active-time measurement, so the historical
“half-day connector” claim remains unproven and is not repeated.

**C6 closure and future-plugin checkpoint (2026-08-23).** A
`DriverManifest` is now the data-only capability projection a future build
description, native-plugin sidecar or out-of-process handshake must produce.
It contains a canonically Connector-id-ordered set of `(ConnectorId,
ConnectorRoles)` entries and rejects duplicate ids; it contains no factory,
function pointer, crate, artifact path, Host, Worker or loader decision. The
manifest can therefore be inspected without executing implementation code.
Its serialized file syntax is deliberately not frozen.

A manifest claim is never an executable binding. After any future loader has
made code available, the resulting factory must still enter the ordinary
`DriverRegistry`; `RuntimeBindings` independently proves the exact id,
required advertised roles and actual bound role views before execution. M1.5
freezes neither a Rust/native ABI nor artifact discovery, trust/signing,
isolation, lifecycle or hot reload. Static registration remains the only
shipped assembly mechanism.

The contributor path is now one path for every physical assembly: Contract
sheet and Registry-free validation, family-owned pure request protocol and
effectful Driver, exact-id factory, real conformance profile, Lab mock factory
over the same protocol, then reduced/combined composition proof. The
WHITEPAPER, active specifications, architecture map and maintained module
diagram use that vocabulary. Historical M0 artifacts retain superseded names
only where they are explicitly marked historical. C6 changes no language,
plan, semantic hash, package, causal trace or format version; M1.5 closes at
this checkpoint.

**Post-closure adversarial hardening (2026-08-24).** The closure review found
no missing abstraction, but it did expose boundary doors that must fail before
effects. A callable Connector without an outbound role now fails Registry-free
plan validation, and the same validation checks the complete effective
Process sheet even for an unreachable Node. Runtime role tags have one
encode/decode authority. A scheduler timeout proves the Driver cancellation
before winning close-once, so a cancellation refusal leaves both call and
deadline replayable rather than stranding a mailbox.

The File Driver rejects Windows drive-relative spellings as `InvalidPath`
before joining them to its deployment root and normalizes an existing
non-file to `NotAFile` even when the host refuses the initial open. Its replace
preparation now copies the payload into, and hashes, only the final operation
descriptor. The interleaving conformance profile requires distinct expected
outputs as well as distinct requests, so a crossed reply cannot pass with an
invisible oracle. Runtime call-family and Dispatcher start caches are retired
when their journal obligation ends. Each family module supplies its own pure
scripted-protocol binder; the Lab composes those binders without concrete-id
dispatch and shares immutable mock trees. The standalone runner exposes File
through explicit `--file-root <dir>` deployment data. These are refusal,
portability, liveness, and assembly corrections only: no language, plan,
semantic hash, package, causal trace, or format version changes.

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

Inbound admission is now a prepare/commit operation. Preparation mints the
final root envelope, including `meta.sys`, without consuming its sequence; the
VM then selects and validates the complete destination owner chain outermost first,
checks mailbox capacity, and only then commits the inbound fact and its route.
A normal transaction follows the same select/check/commit discipline. Thus a
key may honestly read the complete immutable envelope, and failure of a key,
owner value, or instance-bound Connector setting commits no inbound fact,
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

**R7 closure and placement checkpoint (2026-08-22).** The executable M1.4
instance slice is closed under its one-worker contract. The old source forms
remain only as parser recovery nodes explicitly named `Removed*`; `partition`,
Graph/Operator/Connector `per`, join `by`, and targeted `stop` each produce one
`E-INSTANCE` migration diagnostic and cannot enter HIR or plan. No
`PartitionSpec`, old routing key, or v8 decoder path exists; every earlier
categorical version remains refused by number.

Physical placement remains an external lookup after logical selection:
canonical address → fixed virtual shard → current Worker. The public keyed Lab
proof reassigns one shard between two different Host/Worker pairs and observes
the same semantic hash, journal records, canonical record bytes, and
reconstructed replay trace. Neither assignment is an input to compilation,
routing, journaling, or replay. Full Worker leases, fencing, migration and
distributed recovery remain later architecture, not hidden M1 behavior.

The measured unkeyed lookup introduced in R4 is retained. An executable probe
routes twice through an unkeyed child of a keyed Graph and reconstructs the
exact canonical child address independently; the second route proves the fast
lookup. It is a fast path over the same complete `InstanceRef`, not a singleton
authority. The general route path is strengthened so an internal delivery
during Graph Closing continues in an already-active keyed descendant instead
of selecting its successor. R7 moves no semantic hash, package, or causal
golden; the syntax-tree debug golden moves only because its recovery field is
renamed from active-looking `per` to `removed_per`. The full M1.4 move ledger
and named limits are recorded in
[`../architecture/m1.4-exit-report.md`](../architecture/m1.4-exit-report.md).
The final portability gate passes 697 all-feature tests, three `no_std` ISAs,
and the Cortex-M33 spike at 2 753 relay ticks average / 2 986 maximum with the
semantic hash still `419a52a1…`.

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

In particular, a non-zero process code changes from a Driver-classified
failure into an observed terminal fact. Retry or alert policy belongs to the
graph after narrowing `exit`, not to the Process Driver.

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
Leaves use the closed
`scalar | array<scalar> | map<text,array<scalar>>` carrier after the narrow
H4 amendment in D-2026-08-26-2; other composites live in `data`. The
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
Type `media_type?` under D-2026-08-26-3 (superseding this entry's original
`text?` carrier without changing placement or payload coupling); read as
`meta.ext.content_type`. There is no
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
