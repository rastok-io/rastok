# Identity and canonical form

> **Status (2026-09-05): normative.** This document fixes the
> **semantic identity** of a graph, the
> **allocation of `graph_version`**, and the **canonical byte form** of
> journal records.
>
> The registry below retains the executable v7 value registry inside
> categorical plan-package v18. V12 introduced the
> open `media_type` scalar and its constructor, and specifies the M1.4 instance-address
> extension under D-2026-08-18-1. V9 refuses the old unpublished v8
> named-partition draft and carries the parent-linked owner tree, scoped edges,
> typed key/lifetime policy and phased immutable expressions. R3 made
> the separately declared mandatory `(address, generation)` causal facts
> executable and moved trace goldens without changing semantic hashes or plan
> packages. R4 executes every lifetime profile on those facts and deliberately
> adds universal owner lifecycle records without changing a semantic hash or
> plan package. R5 adds bounded Closing facts and the empty
> `ProcessExit::OutcomeUnknown`; existing non-closing traces stay fixed while
> Process semantic hashes/packages move deliberately. R6–R7 move no existing
> semantic, package, or causal artifact; R7 audits the complete M1.4 move ledger and proves a Worker
> reassignment cannot alter canonical replay bytes. M1.5 C1 replaces source
> Connector names with stable versioned ids and required roles, moving
> Process semantic hashes and v10 packages while leaving causal records
> unchanged. M1.6 H1 adds the closed Event/Call/Session exchange tag, moves
> Connector semantic hashes and every package to v11, and leaves pre-session
> causal records byte-identical because the new session keys are omitted.
> H5 first adds source Connector key 27, then the generic inbound Call fold
> under D-2026-08-26-5 with INBOUND key 28 and TRANSACTION key 52. H5.1 binds
> live H1–2 ack traffic without allocating another form. H5.2 permits the
> existing consumed-position key 33 beside cause key 39 and Call-exit key 52
> for a graph response; no key or recipe is added. All three allocated H5 keys
> are omitted from older facts, so no existing causal record moves.
> Final H5 allocates timer kind 8 for a retained inbound-Call request deadline
> and narrows `HttpServerExit` to facts possible after admission. Only the
> representative HttpServer semantic hash moves; the then-current plan v12 and prior records
> remain byte-identical. H6 executes the H1 Session forms and optional timer
> kind 9 without moving an existing artifact; H7's static assembly is wholly
> physical and likewise moves none.
> M1.7 L2 moves every package container categorically to v13 for closed
> Graph-aspect contracts and explicit Graph-handler ownership. Existing
> semantic hashes and every causal record remain byte-identical; newly checked
> Loop programs seal their contract under `rastok.aspects.r1`. L3 adds no
> identity; L4 allocates only new Loop causal forms under domains `0x05`/`0x13`
> and transaction key 53, leaving every pre-Loop artifact byte-identical.
> L6 moves every package container to v14 for optional proof certificates.
> Existing fuel-only Loop source retains `rastok.aspects.r1`; a declared
> proof adds `rastok.loop-proof.r1`. No existing causal record moves.
> L7 adds public composition evidence and fixes runtime expression bindings,
> but moves no semantic hash, plan package, or causal record.
> M1.8 A2 moves every package container categorically to v15 for canonical
> schema-policy side-tables. Unannotated bodies and semantic hashes remain
> byte-identical; annotated programs alone gain `rastok.metering.r1`. No
> causal record moves.
> M1.8 A4 reuses the existing categorical expression-path form for exact boot
> references. Physical values and profile facts move no graph or causal
> identity; their immutable run-header attestation gates replay instead.
> D-2026-08-14-1
> transports the canonical `DataType` registry, complete resolved port types
> and structural/tagged unions together. Decode re-derives the effective table
> from the plan constraints and requires exact agreement with the transported
> claim.
>
> D-2026-09-05-1 and R4 allocate the v16 ledger in §1.1. The current
> package tables live in [plan-package.md](../architecture/plan-package.md);
> §3 includes r2 journal key 55 and retained timer key 56. Key 53 stays
> historical, never reinterpreted. [R9 qualification](../architecture/rework-r9-qualification.md)
> records the single blessing and unchanged causal witnesses.

## 1. The semantic hash

The identity of a graph is `blake3(canonical HIR)` — the full 256-bit
digest. The canonical HIR is the program **desugared, name-resolved and
typed, before boot-time const-eval**.

```
source .rkg ──parse──► AST ──desugar/resolve/type──► HIR ──canonicalize──► bytes ──blake3──► semantic hash
                                                        │
                                                        └── env("X") / secret("p") are still EXPRESSIONS here
```

**Insensitive to** : comments and formatting (do not survive the parse) ;
file paths, file order, declaration order (top-level declarations are
sorted by (kind, name) before hashing) ; sugar spelling (a form hashes as
its desugared equivalent) ; **omission of a contract default**
(D-2026-08-12-1: the checker injects every omitted `Defaulted`
attribute — the profile selector's word included — into the
effective HIR BEFORE canonicalization, so `mode` omitted and
`mode = job` are one identity ; this merges only the EXACT spelling
of the default — `duration("60s")` and `duration("1m")` keep two
trees) ; **the entire lab** (a lab never alters the
production graph).

**Sensitive to** : topology (the parent-linked Graph/Operator/Connector owner
tree, scoped PhysicalEdges, RouteRules, ports, edge configs), resolved
`instance.key` and `lifetime` attributes; owner `val` expressions and every
effective Connector attribute's selected evaluation site; Operator Dispatch,
the order of linear statements (`emission_index` = source order), the order of
ports in a join, and the order of open-match arms; every Rastok expression
(hashed as its canonical expression AST); resolved logical types and complete
port tables, Connector `ContractId` values and required roles, attributes,
annotations, `variant` clauses ; `var` defaults
(the recovery fold reads them) ; guest-language block bodies (opaque — raw bytes
after common-indentation stripping) and the pinned guest language,
frontend/compiler/lowering profile, ABI and `GuestExecutor` identities.

Rules :

- Unit-global `val`s are plan constants, folded and substituted with their
  checked type before production HIR (D-2026-09-14-2). They have no serialized
  binding table. An unused or Lab-only constant contributes no bytes. A scalar
  literal reference substitutes that same literal tree, so factoring it into a
  global preserves hash, package and causal traces. Composite/computed constants
  retain explicit type witnesses; this does not promise that arbitrary literal
  rewrites or constant arithmetic share an identity. Owner `val`s keep their
  existing expression/phase identity rules. Reordering accepted declarations
  does not change identity; initializers must still follow their dependencies.
- `env()` / `secret()` hash **as expressions** : the same source has the
  same identity in dev and prod ; values live in the deployment profile,
  never in the identity.
- A future precompiled `GuestProgram` hashes its content digest plus its pinned
  ABI/profile identities in place of a source block. Changing a guest
  toolchain or executor therefore changes the semantic hash and allocates a
  new `graph_version`; Wasm syntax remains out of scope here.
- **No α-equivalence** : node names are `node_path`, hence id material —
  renaming a node is an identity change.
- Extension order, closed-match arm order, source/destination-set order, and
  explicit boundary spelling are erased before hashing. Ordered emissions and
  open predicates are not.
- Identity = canonical-form equality, nothing more ; semantic
  equivalence of different programs is out of scope.

**B5 connector-contract move (2026-08-15).** A resolved Connector's complete
port rows — name, direction, unwired policy and effective logical type — are
canonical HIR identity. `err_codec` is a normal `Defaulted` attribute, so its
omission and `err_codec = lines` are identical after pre-hash injection. The
former `error_set` identity slot is retired rather than retained beside the
typed port model. Consequently Process graphs move once (echo/spike
`a7511135…`, JSONL `4eacb55c…`, compiler probe `f068bcd1…`). The
connector-free gauge semantic hash remains `0d3ad43d…`. B5 temporarily made
an implicit Operator runtime-fault row explicit only in the package; the final
M1.3 review removed that invented row and the synthetic fault emission. Echo,
JSONL and gauge package bodies move back to the port tables their canonical
HIR hashes actually describe, while those hashes and causal traces remain
unchanged. These are pre-public checkpoints; v7 still seals the full tagged
types without renumbering their logical registry.

**B6 construction checkpoint (2026-08-15).** A checked `Case { ... }` hashes
as its resolved name-stable tagged-union constructor and a contextual
structural-union write hashes as the explicit resolved `as` node inserted by
the checker. `err` contributes no error-specific statement tag: it is the same
ordered port emission as `out`, `exit` or an application-named output. These
nodes had no v6 package spelling and were categorically refused until B8;
therefore no previously valid v6 program, semantic hash or golden moved at
B6. V7 now carries them.

**B7 completion checkpoint (2026-08-15).** A Process invocation now publishes
unit-framed `out*`, independently framed `err*`, then one exact terminal
`exit`, all in one ordered completion transaction. The resolved Process port
types and effective `err_codec` therefore move Process identity deliberately:
echo/spike `e322906a…`, JSONL `87d13844…`, and compiler probe `77d326e3…`.
Their package and causal-trace goldens are re-blessed because both the plan
contract and closed-call/output record shapes changed. The connector-free
gauge hash remains `0d3ad43d…`, proving the move is scoped. The raw v6 package
represented `Process.exit` as `variant`; B8 replaced that transport bridge
with the exact tagged union in v7.

**B8 package checkpoint (2026-08-15).** V7 transports the complete canonical
type tree, expression forms and total resolved-port table. The semantic
representation bridges disappear: `Process.exit` is exact `ProcessExit`,
`decimal()` is `number`, and an integer literal outside `int32` is `int64`.
This deliberate pre-public move changes echo/spike to `514d4ab7…`, JSONL to
`18dda78c…`, and the compiler probe to `fa5abd79…`; connector-free gauge stays
`0d3ad43d…`. Package goldens move because the body adds the resolved table and
complete type constructors even when the graph's semantic hash is unchanged.

**M1.4 R5 Closing checkpoint (2026-08-21).** An explicit `closing` policy is
semantic identity: its ordered `drain_for` and `settle_for` expression trees
are sealed under the `rastok.closing.r1` extension, while absence contributes
no bytes and cannot collide with a written policy. Adding the empty
`OutcomeUnknown` case to `ProcessExit` moves every Process graph once:
compiler probe `29c69b70…`, JSONL `a058f1aa…`, and echo/spike `419a52a1…`.
Their v9 package bodies move with the exact tagged union; connector-free gauge
remains `0d3ad43d…`. Existing traces do not move merely because a type grew.
When Closing executes, transaction key 49 and timer kinds 6/7 make its new
causal facts visible; no existing non-closing golden gains those optional
fields.

**M1.4 R6 join checkpoint (2026-08-22).** A join activation seals its ports in
source-declared order; `(a, b)` and `(b, a)` are different programs. Optional
`within` and `last` bounds are identity, while declaration order between
distinct handlers remains erased by canonical activation key followed, when
equal, by a key derived from the complete handler. These
bytes occur only for a join or a newly public nested Graph, so every existing
semantic hash, package, and causal trace remains fixed. When a join fires,
transaction key 34 records exactly one contributing envelope id per declared
port in that same order; key 33 independently remains canonically sorted by
cursor.

**M1.5 C1 Connector-identity checkpoint (2026-08-22).** The source contract
symbol resolves to one canonical versioned id (`ConnectorId` at that
checkpoint, now `ContractId`); that id and the
contract's closed inbound/outbound role requirement replace the source name
in canonical HIR and format v10. Process identities therefore move once:
compiler probe `29c69b70… → 52406fcf…`, JSONL
`a058f1aa… → 9643616e…`, and echo/spike
`419a52a1… → 190f0efc…`. Connector-free gauge remains
`0d3ad43d…`. V10 package containers move; existing causal traces do not,
because Driver selection is neither a causal event nor a placement fact.

**M1.6 H1 exchange checkpoint (2026-08-25).** The closed exchange lifecycle
joins the Connector's canonical HIR identity and v11 plan row. Compiler probe
moves `52406fcf… → 2933618d…`, JSONL moves
`9643616e… → 036f7ea5…`, and echo/spike moves
`190f0efc… → e50026ee…`; connector-free gauge remains
`0d3ad43d…`. All three package goldens move categorically from v10 to v11,
including gauge's container, and the spike embeds the 974-byte echo package.
Keys 26, 50 and 51 are optional and absent from every existing Call/Event
fact, so all four pre-H1 causal traces remain byte-identical. The new session
identity domains `0x11` and `0x12` are pinned below and contain no socket,
protocol-version or placement fact.

**M1.6 H2 network-sheet checkpoint (2026-08-25).** D-2026-08-25-2 adds four
previously invalid Connector contracts and reserves their standard protocol
metadata. It changes no identity recipe and no categorical field: a new
network graph seals its exact stable contract id, effective defaults, ports,
types and settings through the existing v11 forms, while a negotiated HTTP
version remains an observation. Every pre-H2 semantic hash, package body and
causal trace is therefore byte-identical; no golden is re-blessed. The
connector-free gauge remains the negative scope witness.

**M1.6 H3 hosted-engine checkpoint (2026-08-26).** Cargo features select the
bounded pump and private HTTP/QUIC/TLS/WebSocket libraries; none is a semantic
or causal fact. Every pre-H3 hash, package and trace remains byte-identical and
the connector-free gauge remains the negative scope witness. H4–H6 network
facts use H1's already allocated Call/Session forms rather than
extending a package merely to name a library or negotiated wire version.

**M1.6 H4 live-HttpClient checkpoint (2026-08-26).** The live Driver consumes
the exact H2 sheet and H1 Call forms. Prepared request bytes already own the
OUTBOUND digest; selected HTTP version and response headers/status are external
observations on newly minted output envelopes. D-2026-08-30-3 later narrows
“request bytes” to the complete logical descriptor when an attested secret is
used: its logical reference is sealed while physical material is fixed by the
run header and overlaid only inside the authorized Driver. The new metadata-map carrier
reuses value tags 18/19/8 and existing scalar tags, so no old metadata value
changes bytes. H4 therefore moves no package, semantic hash or pre-existing
causal trace; all goldens and the gauge witness remain byte-identical.

**M1.6 H4.1 HTTP-schema checkpoint (2026-08-26).** D-2026-08-26-3
allocates categorical package v12 because `media_type` becomes a first-class
logical type (type-tree tag 18, semantic-HIR type tag 26) and
`media_type(text)` becomes a first-class expression function (plan and
semantic-HIR function id 6). `Value::Text` remains the sole materialized
carrier, so no value tag moves. Every package container moves categorically
from v11 to v12. Semantic hashes move only where a checked type, metadata
write, frontend content type, or HTTP sheet now contains `media_type` or the
typed `headers` / `custom_headers` split. Existing causal records move only if
the semantic hash sealed in their envelope identity moves. The connector-free
gauge remains the negative scope witness.

The landed ledger is exact. A source witness whose whole payload is
`base64!("image/png") { AAEC }` moves `fc8cddb6… → ee1dab9e…` because the
checked content-type write now carries the new logical type. The first typed
HttpClient-header witness is new identity `c66bd9da…`, not a reinterpretation
of the removed untyped header map. Network witnesses that use none of the new
typed fields remain HttpClient `481662aa…`, HttpServer `63a257e2…`,
WebSocketClient `d4f92e15…`, and WebSocketServer `d36bcbf4…`. Compiler probe
`2933618d…`, JSONL `036f7ea5…`, echo/spike `e50026ee…`, and gauge
`0d3ad43d…` remain fixed. The three package goldens — echo 974 B, JSONL
1 018 B, gauge 313 B — plus the spike's binary echo copy change only their
v11→v12 envelope version and checksum; no stored causal trace moves.

**M1.6 H5 generic inbound-Call checkpoint (2026-08-26).**
D-2026-08-26-4/-5 allocate optional INBOUND source key 27, inbound-Call key
28, ordinary TRANSACTION terminal key 52, and internal lifecycle tag 9
(`reply`). No source handler may carry that tag. The fields occur only for new
Connector-origin facts, so every existing package, semantic hash and causal
trace remains byte-identical; no golden is re-blessed. The new fake-HttpServer
proof pins keys 28/52 directly and keeps the connector-free gauge as the scope
witness.

**M1.6 H5.1 live-ack checkpoint (2026-08-26).**
D-2026-08-26-6 maps real HTTP/1.1 and HTTP/2 requests onto the already
allocated source/Call/terminal forms. Listener addresses, negotiated version,
Hyper/Rustls state and physical capacity are observations or deployment
material, never plan or causal identity. No package, semantic hash, existing
trace, root recipe or golden moves.

**M1.6 H5.2 graph-response checkpoint (2026-08-27).**
D-2026-08-26-7 allocates no field. A direct response uses the existing route
key 38 on its producer, then the existing consumed-position key 33 and root
cause key 39 beside inbound-Call exit key 52. Fixed ack retains the zero-cursor
form. The keyed-source proof pins the response route to the opening
`InstanceRef` even when transformed response data would derive another key.
No existing package, semantic hash, causal trace or golden moves.

**M1.6 H5 closure (2026-08-27).** D-2026-08-27-1 removes the impossible
pre-admission `MalformedRequest` and `AuthFailed` branches from
`HttpServerExit`. The representative HttpServer witness therefore moves
`63a257e2… → b83fcf2d…`; HttpClient `c66bd9da…`, WebSocketClient
`d4f92e15…`, WebSocketServer `d36bcbf4…`, compiler/echo/JSONL and the gauge
`0d3ad43d…` remain fixed. This is a semantic and package-body move only for
graphs whose resolved HttpServer exit type changes; the categorical package
format remains v12. New respond-mode INBOUND facts may add timer kind 8 with
the `InboundCallRef` as reference. It is omitted from ack and every old fact,
so existing causal traces do not move.

**M1.6 H6 WebSocket-Session checkpoint (2026-08-27).**
D-2026-08-27-2 executes the H1 Session forms without extending the plan or an
existing causal record. HTTP/1.1 Upgrade, RFC 8441 and RFC 9220 are physical
bindings excluded from `SessionRef`, semantic hash and package bytes. New
Session facts use the H1 reference/write domains, lifecycle `l:exit`, and
optional timer kind 9 for handshake timeout; those fields are absent from all
pre-H6 facts. Every existing semantic hash, package golden and causal trace is
therefore byte-identical. Echo/spike remains `e50026ee…` and connector-free
gauge remains `0d3ad43d…`.

**M1.6 H7 static-assembly checkpoint (2026-08-27).**
D-2026-08-27-3 makes Cargo features, linked factories and the physical File
root inputs to `hosted_registry`, never semantic facts. `DriverManifest` is a
data-only projection of that registry and does not enter a plan. Reduced and
full images therefore accept or refuse the same package according to linked
capability without changing its hash, bytes or history. No identity register
moves at H7.

**M1.7 L2 aspect-transport checkpoint (2026-08-28).**
D-2026-08-28-2 widens the owner and handler grammars in categorical v13:
nested Graph boundaries, one closed canonical aspect list, tagged
Node-or-Graph handler ownership, and direct-child output activation. Every
v12 container moves because the version and checksum are categorical; v12 is
refused. Existing programs keep their semantic hash — echo/spike
`e50026ee…`, JSONL `036f7ea5…`, and connector-free gauge `0d3ad43d…` — because
root boundary data is not duplicated and empty derived owner lists are not
semantic source claims. Their v13 packages are respectively 988 B, 1,032 B,
and 323 B. A new Loop seals its checked `LoopSpec`, contributed
ports and Graph handlers under `rastok.aspects.r1`. L2 emits no Loop record,
so all stored causal traces remain byte-identical.

**M1.7 L3 finite-control checkpoint (2026-08-28).** Several guarded Graph
handlers may now share one activation only when their conditions are proved
disjoint and exhaustive. Source semantic identity tie-breaks one equal
activation by the domain-separated BLAKE3 digest of the complete semantic
handler; the shipped table uses the complete canonical transported handler
bytes. Both keys are derived from the same already sealed meaning, so source
declaration order carries no identity and no new claim travels. The proof
itself is derived from handlers, edges, port types and Connector exchange
facts. Every pre-L3 program keeps its semantic hash, v13 package bytes and
causal trace; L3 emits no Loop record. The connector-free gauge remains the
negative scope witness.

**M1.7 L4 Loop-fold checkpoint (2026-08-28).** L4 allocates envelope domain
`0x05` for manager-produced immediate re-entry, reference domain `0x13` for
`LoopRunRef`, and optional TRANSACTION key 53 for the closed Loop fact. These
forms occur only when a new Loop executes. Every pre-L4 semantic hash, v13
package and causal trace therefore remains byte-identical; no existing golden
is re-blessed. One run derives from `(instance_address, generation,
external_root_id)`. One re-entry derives from `(run_ref, next_iteration,
selected_next_id)`. The journal folds open/next/close/expired facts and rejects
any skipped, duplicate or post-terminal transition; Worker RAM contributes no
identity or fuel.

**Transaction-time context checkpoint (2026-08-28).** D-2026-08-28-5 adds
optional TRANSACTION key 54 for the signed nanosecond logical instant fixed for
a handler that can observe `now()`. Existing expression transport already
represents the reserved context path, so v13 and semantic identity do not move.
The key is absent from every handler that cannot observe time; all existing
causal traces therefore remain byte-identical. Scheduler arming and due dates
remain derivable annotations. Only the explicit value allowed to influence a
transaction is sealed.

**M1.7 L5 dated-admission checkpoint (2026-08-28).** Key 53 gains the
extend-only `admit = 5` subtag. `Next` remains the selected-state and fuel
transition; `Admit` is the later exactly-once `l:next` crossing and seals any
spatial routes under the already published key 38. Timer kind 5 remains the
sole delayed obligation and continues to reuse the re-entry envelope id. The
new subtag occurs only in newly executable Loop histories: categorical v13,
semantic hashes and all non-Loop causal traces remain byte-identical.

**M1.7 L6 checked-measure checkpoint (2026-08-28).**
D-2026-08-28-7 moves every package container categorically from v13 to v14:
`LoopSpec` key 1 becomes optional fuel and new key 3 carries the checked
measure, optional invariant and sorted decrement witness. V13 is refused.
The witness is re-derived from canonical handlers and is not semantic source
identity. Fuel-only Loop programs retain their exact
`rastok.aspects.r1` stream; proof-bearing programs add
`rastok.loop-proof.r1`. Echo/spike `e50026ee…`, JSONL `036f7ea5…` and gauge
`0d3ad43d…` remain semantic witnesses, and all existing causal traces remain
byte-identical. The v14 echo/spike, JSONL and gauge containers remain 988 B,
1,032 B and 323 B respectively; their categorical checksums are
`afcc22ac…c0ebc`, `1ca982c4…e54a06` and `cf8e9ac4…399bc5`. The complete bytes
are pinned by the package goldens and the Cortex-M33 measurement below.

**M1.7 L7 closure checkpoint (2026-08-28).** D-2026-08-28-8 adds no
identity-bearing fact. The finite control walk follows descendant Nodes in the
already flattened scoped DAG, and the public Lab exercises already published
Loop, instance, Call and Closing forms. Categorical v14 bytes and checksums,
semantic hashes and every existing causal trace are therefore byte-identical
to L6. Echo/spike remains `e50026ee…`, JSONL remains `036f7ea5…`, and the
connector-free gauge remains `0d3ad43d…`. No golden is re-blessed for L7.

**M1.8 A2–A3 data-policy checkpoint (2026-08-30).**
D-2026-08-30-1 moves every package container categorically from v14 to v15.
Sparse policy tables sit beside, rather than inside, `DataType`; the type and
value tag registries do not move. `SchemaPath` segment tags are extend-only:
`1 = field`, `2 = array element`, `3 = map key`, `4 = map value`,
`5 = structural-union branch`, `6 = tagged-union case`. Annotation tags are
likewise extend-only: `1 = sensitive` (levels 1 internal, 2 confidential,
3 restricted), `2 = secret`, `3 = measure`, `4 = cost` (basis 1 provider-
reported, 2 pricebook-calculated; optional currency). D-2026-08-30-2 appends
`5 = personal` with a closed bit mask (`1 = Article 9 special category`,
`2 = Article 10 criminal offence`, `4 = subject identifier`). No existing
tag, set order or package version moves. Root is the empty path.

The version/checksum-only move leaves echo/spike at 988 B, JSONL at 1,032 B
and gauge at 323 B. Their v15 categorical checksums are respectively
`73ea33a7b0a9a4f25a109748b4a012c885d04cdf5aab91cc765052195394847a`,
`3805f1eb650f74d705eaaa83ef7af48f5df85ebd6c559d130414be813008bd26`
and `a7e7e5743273b5a31ba3c3c616bde8fd260750acce753c7c750ea3186a45adcb`.
Echo `e50026ee…`, JSONL `036f7ea5…` and gauge `0d3ad43d…` remain exact
unannotated semantic witnesses. Only an actual resolved table adds the
`rastok.metering.r1` stream; its first pinned semantic witness is
`f984f003953161e8fa93025787bd92a44fd18a8615aee1a0fbe4b9f1ba22defe`.
Every existing causal trace remains byte-identical. Registry-free validation
re-derives every local declaration. A3 adds no new wire tag or identity
stream: it derives effective resolved-port policy through exact copy,
projection, construction, calculation, guards, state and Loop recurrence, and
compares that result with the already transported resolved table. Raw source
rows are never rewritten with derived verdicts. Annotated immutable bindings
remain as paths in executable trees so their policy dependency is provable;
this may change only annotated package bodies. Unannotated bodies and the
three semantic witnesses above remain exact, and no causal trace moves.
The additive A3.1 privacy registry uses the existing policy stream only for
programs that write `@personal`; all earlier annotated witnesses remain exact.

### 1.1 Plan v16 ledger

Plan v16 is one categorical move, not a sequence of partially readable
packages. R4 allocated its exact tags after R3 stabilized the source/HIR
algebra; R9 blessed the three independently validated package witnesses once. The movement set is:

- `ExtensionSpec` replaces active Aspect transport; historical tags and
  `rastok.aspects.r1` keep their old meaning;
- recursive `Dispatch` replaces a flat handler body;
- `RouteRule` and canonical disposition sets are distinct from
  `PhysicalEdge`;
- manager-command destinations and Loop r2 Produced/Consumed facts;
- exact `OpeningAdmissionRef`, `LoopRunRef`, and routed LoopContext;
- retained timer schedules, key 56, outside the causal projection as decided
  in D-2026-09-07-2; allocation is part of this cutover, not a second R7 bump;
- productibility evidence re-derived by the validator;
- executable ordered multi-emission;
- removal of Graph handlers, `ChildPort`, old Loop `next` port,
  `RootSink`, `UnwiredPolicy`, and the contract `unwired` field;
- the Contracts of six standard Connectors move from `@1` to `@2` because their
  production/cardinality/terminal/effect-stage contracts change.

The owner table, OwnerPath, envelope/value/DataType registries, three causal
record kinds, instance/lifetime/Closing identities, roles, exchange kinds,
Connector `behavior`, and existing timer meanings remain. `node::Type`
was not transported and therefore has no wire tag to remove.

Canonicalization rules:

- Extension lists, closed-match arms, source sets, destination sets, and each
  RouteRule disposition set are sorted and duplicate-free;
- open-match predicates, linear statements, join ports, and multi-emission
  targets retain source order;
- explicit current-boundary names and removed dot sugar must produce the same
  HIR bytes;
- manager commands and physical deliveries inhabit distinct identity domains;
- `CommandProduced` and `CommandConsumed` are distinct append-only facts;
- no retired tag, key, or domain is assigned another meaning.

The additive semantic HIR streams are `rastok.dispatch.r1` for decisions outside the exact v15 linear
projection, `rastok.routing.r1` for case selection and manager destinations,
and `rastok.loop.r2` for command-based Loop owners, their entry, terminals,
fuel, delay, and authored measure/invariant. Ordinary unconditional fan-out
keeps the existing edge stream: writing a destination set or its separate
arrows has the same identity. Universal destinations are merged into each
case row, and case/destination order is canonical. Changing a case's target
or a command kind changes the hash even when physical edges are identical.
Old Loop domains remain historical. This v16 checkpoint has no active r1 fold
or compatibility execution path; v17 extends its ledger below.

The re-bless set is determined before R4. All package containers move
categorically. Semantic hashes move only for programs affected by the new
meaning. Causal goldens move only when they exercise changed facts. These are
the exact pre-bump baselines used to prove the scope:

| Witness | Semantic hash | v15 body size |
|---|---|---:|
| echo / Cortex-M33 semantic witness | `e50026eeb0b8dc77fe2f5e52de6dfaa33b50cc351211cd87c21081d28aa43709` | 907 B |
| JSONL Process witness | `036f7ea5346fac883a3bad155fbdf41b303f6e54bef93f8c850ff5f138f769de` | 951 B |
| connector-free gauge | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | 243 B |

The two Process semantic witnesses move with Contract `@2`; the gauge hash
stays identical. Body sizes become 939 / 983 / 249 B. The complete hashes,
checksums and unchanged causal seals are in the
[R9 ledger](../architecture/rework-r9-qualification.md), recorded before blessing.

### 1.2 Plan v17 — Loop host terminals

D-2026-09-08-1 removes Loop's contributed expiry port. LoopSpec keys 1–5
retain their meaning; key 6 optionally carries the resolved exhaustion
`[port, complete_type]`. It is absent only for proved-impossible exhaustion.
The Loop-specific semantic stream also seals that target and type. Operators
and Graphs declare ordinary terminal unions; a Call's effective terminal pin
extends its unchanged `@2` Driver Contract with a manager-only case.

No r2 fact, command, run/admission domain, timer or causal record shape changes.
An Expired emission uses its actual host port in the ordinary emission-id
recipe. Recovery verifies that port against the plan. Non-Loop semantic
streams, package bodies and causal seals stay exact; only their categorical
version/checksum changes. V16 packages refuse by version, never reinterpretation.
The [v17 ledger](../architecture/loop-host-terminal-v17.md) lists evidence and
the single fixture blessing separately from the historical v16 checkpoint.

The complete pre-allocation register and re-bless protocol are in
[rework-r0-identity.md](../architecture/rework-r0-identity.md).

### 1.3 Plan v18 — exact-instance closure

D-2026-09-10-1 is the explicit exception to the opening freeze: generic
instance closure, not Process service. Package v18 allocates route destination
tag 4 to `[4, lexical_graph_owner, target_owner]`. The lexical boundary routing
inventory is mandatory even in a flat command-bearing Graph. Core independently
checks the scope and exact direct-child authority. Tags 1–3, Contract IDs,
ordinary `stop`, Loop r2 and all existing port identities keep their meaning.
V17 packages refuse by version, not reinterpretation.

Only command-bearing programs gain the framed routing contribution. The echo,
JSONL and gauge package bodies, semantic hashes and causal traces must remain
byte-identical. Their outer version and checksum, including the spike's echo
copy, are the only witness updates authorized by this allocation.

The new command identity domain `rastok.instance-stop.r1` uses byte `0x23`,
framing source kind/port, exact source/scope/target addresses and generations,
and the causal envelope ID. Command packets use causal key **57**, absent when
unused. Packet key 1 carries strictly ID-sorted productions; key 2 carries the
winning consumption, alone. The target-owned `l:closing` retains consumption;
the producer's record retains only production. A competing retained closure
settles redundant requests without fabricating another transaction.

Diagnostic recording **v2** adds the lossless optional packet at the end of
INBOUND and TRANSACTION rows. Older recordings refuse by version. No other
record species, close-scope ownership, timer schedule or existing seal changes.
The [implementation ledger](../architecture/instance-stop-command.md) records
qualification separately from this allocation, before fixture re-blessing.

### 1.4 Resident Process — Contract move without a package move

D-2026-09-14-1 admits the service profile as `rastok/process@3`, with both
roles and Session cardinality. The default job remains `rastok/process@2`.
The resolved exact id, roles and exchange already travel in v18, so no
package or journal allocation is needed. A profile id cannot alias the base
contract or another catalogue entry.

Existing job defaults, semantic streams, package bodies and causal witnesses
remain unchanged. New resident programs carry their distinct Contract and
`max_frame` setting; the gallery's dialogue traces are initial witnesses,
not a re-blessing of echo, JSONL or gauge. The opening Session intent seals
the resolved process descriptor and its first input; write intents seal later
frames. All existing Session identity and close-once recipes retain their
meaning. See the [service ledger](../architecture/process-service.md).

### 1.5 Optional member presence — expression allocation inside v18

D-2026-09-14-3 appends pure function id **7**, `has`, to the transported
expression and semantic-HIR function tables. Its two internal arguments are
the receiver expression and literal text member; the absent final member is
never evaluated. Registry-free validation checks that shape and re-derives
the receiver and result types. This allocates no expression tag, plan field,
Contract id or causal key; the package remains v18. A reader without function
7 refuses it by name rather than treating it as another function.

Only programs using presence inspection gain these expression bytes. Existing
echo, JSONL and gauge packages, hashes and causal traces remain unchanged.
No witness re-blessing accompanies this approved opening-freeze exception.

## 2. `graph_version` and the manifest

`meta.sys.graph_version` is a small integer. It is a
**flyweight alias** of the semantic hash ; the (seq ↔ hash) mapping is
the *manifest*.

**Allocation, at boot** : `graph_version` is a **monotone
`u64` build/deploy number, always increasing — a rollback receives a
NEW number**. Chronology is the number's whole justification. Its
manifest row points at the *same*
content hash as the earlier deployment — identity lives in the hash,
order lives in the number. `seq = max + 1`, append-only, never
renumbered, never reused.

**Embedded / `durability = none` mode** (which includes every
simulation) : the manifest is re-seeded at each boot, so
`graph_version = 1` deterministically — golden traces stay stable and a
golden diff keeps meaning a behavior diff. (The canonical form's
`graph_version` is likewise the constant 1 in embedded/sim, §3.)

**Persistence — the manifest is a first-class append-only ledger.**
It cannot be derived only from prunable segment headers, because
retention could delete the sole copy of an old row. The manifest is a
small non-prunable file
beside the journal (`manifest.rkm` : rows `(graph_version u64,
content_hash, semver, commit, schemas-ref)` — tens of bytes per
distinct graph ever deployed, kept forever). Segment headers still
stamp `(seq, semantic_hash)` as an **integrity cross-check**, never
as the source ; reconstruction-by-scan survives as a *repair* tool,
not the authority. Per-record physical encoding is free to factor
the integer out (an encoding concern, not an identity concern).

**Replay** : a record's `graph_version` resolves through the
**`manifest.rkm` ledger** to a hash, compared with the loaded graph's
hash (segment headers carry `(seq, hash)` only as a local
cross-check, never as the resolution source). Equal →
replay ; different → typed refusal **`E-REPLAY-VERSION-MISMATCH`**
(see [diagnostics.md](diagnostics.md)) — never a silent divergence.
Migration of a journal onto a modified graph is deferred.

**Retention** : journal retention follows the bounded/configurable
model ; the manifest ledger is the kept-forever
piece and is never pruned (~40 bytes per *distinct* graph ever
deployed — a ledger of dozens of KB after a century of daily
deploys). The schemas needed to decode any historical
JSON/Avro/Arrow-backed zone are referenced from manifest rows and
retained with them.

Retaining a schema makes historical bytes readable; it does not make a
new graph accept the old logical type. A V1→V2 migration is an explicit
typed Operator which emits a new envelope and normal lineage. It is not
a package reader and does not rewrite old records. Plan-package v6→v7 is
a separate categorical transport change (D-2026-08-14-1).

## 3. The canonical record form

These tables describe the current causal projection, established at v16,
unchanged by v17, and extended sparsely at key 57 by v18. Historical key 53
is retained in the registry solely to prevent reuse; active Loop state uses
key 55. Required timer dates are retained separately at key 56 and are not
part of the causal seal.

Two byte formats exist and are never the same object : the **durable
journal encoding** (free to evolve) and the **canonical form**
below (fixed ; what golden traces and cross-version replay compare).
Comparisons — including the golden seal — run over the canonical
projection of the causal core, never over storage bytes.

**Encoding : deterministic CBOR — RFC 8949 §4.2, cited in full** :
shortest-form integers, definite lengths only, and the fixed unsigned
keys of record maps emitted in ascending table order (which is their
canonical encoded-key order). Pair-list sub-forms name their own
order: tag-17 struct fields and state-delta paths use UTF-8 name order;
tag-18 Rastok maps use each key's canonical `tagged_value` bytes;
metadata key 9 uses UTF-8 `(zone, field)` order.
**Floats are the exception to shortest-form : a value's
width is FIXED by its `type_tag` — `float` is always IEEE-754 binary32,
`double` always binary64 — so `1.5` is never silently narrowed to
binary16 by one implementation and not another.** `NaN` is the one
canonical quiet NaN (all-payload-zero : `0x7fc00000` / `0x7ff8…0`) ;
`-0.0` is preserved distinct from `+0.0`. Record-map keys are **unsigned
integers** from the tables below; since keys are fixed, emission in table order satisfies the
sorting rule at zero runtime cost. An absent field is an **omitted key**,
never a null. The fast seal of a trace is `blake3` over the concatenated
canonical records ; record-by-record comparison exists to *explain* a
failure (`rastok trace dump` renders the form for review — derived,
non-normative).

**Per-species tables.** A single flat table would conflate the three
record kinds and omit fields that distinguish them : two semantically
different journals could project to the same bytes. The canonical form is a
**shared spine + one extension table per kind**, so that *every
causal fact a record carries has a key*. All tables are extend-only
(append, never renumber, never reuse) ; an absent field is an
omitted key.

**The envelope spine** (keys `1–9`) — how any envelope referenced by
a record is canonicalized (its full `meta.sys`, per envelope.md §1) :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 1 | id | bytes(16) | blake3 first 128 bits (`uuid128`), per envelope.md §2 |
| 2 | parent_id | bytes(16) | omitted at a root |
| 3 | origin_id | bytes(16) | |
| 4 | role | text | value domain per envelope.md §1 ; omitted when `default` |
| 5 | graph_version | uint (u64) | constant 1 in embedded/sim (§2) |
| 6 | — | — | RETIRED (extend-only: never reused). Held `mime_type` until D-2026-08-10-4 moved the content type to `meta.ext.content_type`, sealed in key 9 like every ext field. |
| 7 | tier2 | array | Tier-2 birth stamps (`saga_*`, `loop_*` — envelope.md §1.2), present only when born in scope ; the **sorted pair list** of the sub-form below — never `map` ; keys extend-only |
| 8 | data | array | the tagged payload `[encoding_tag (uint), bytes]` (sub-form below) — opaque, zero-copy, compared as bytes and never re-encoded; an outcome/exit payload is an ordinary envelope, no separate key |
| 9 | app_meta | array | the causal writable metadata — a **canonical sorted list of triples `[zone (text), field (text), tagged_value]`**, sorted UTF-8 by `(zone, field)` ; `zone` ∈ {`app`, `ext`} (D-2026-08-10-1 — root customs died), `field` is the flat dotted key (`audit.origin`) ; sealed because a handler reads them ; omitted when empty |

**Record — shared header** (key `10` ; virtual time is an annotation, below) :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 10 | kind | uint | 1 = INBOUND · 2 = TRANSACTION · 3 = OUTBOUND |

**A raw scheduler timestamp is NOT generally a sealed field**, consistently with
simulation-semantics §6.1, which excludes an obligation's arming and due
times from the fingerprint. A timeout firing at t = 48 h *is* behavior — but
that behavior manifests as a **record that exists in a position**
(the dated-event firing produces its TRANSACTION), and the absolute
µs value is **derivable from (journal, state)** (delivery-semantics
§4 : every dated obligation is derivable). So virtual time is
recorded as an **annotation** (available to `trace dump`, re-derived
on replay), never a compared byte of the seal. Two runs differ in the
seal because their *records and orders* differ, not because a timer annotation
differs. The one deliberate distinction is a processing-time tumbling box:
its canonical box start identifies the selected logical instance and therefore
travels inside the sealed `instance_address` below. That is a journaled routing
fact derived once at admission, not a general license to seal wall-clock or
timer fields. The other deliberate distinction is `now()`: when a handler can
observe the transaction-fixed logical instant, key 54 seals that explicit
input. Omitting it would allow two different control-flow inputs to share one
causal record.

**INBOUND** (kind 1 — an envelope entered) — keys `20–28` and sparse `57`:

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 20 | envelope | map | the spine (keys 1–9) of the entering envelope |
| 21 | connector_path | text | the ingress node's root-relative path (root recipe input, envelope.md §2) |
| 22 | inbound_seq | uint | the atomic sequence for `(connector_path, replica)` — minted by the owning shard's leader ; root recipe input |
| 23 | replica | uint | the logical inbound Connector replica in `InboundEnvelopeProvenance`; sealed so the record autonomously reconstructs `(connector_path, replica, inbound_seq)`. It may be allocated by the leader responsible for that inbound Connector, but is never the target-routing shard and never changes when a destination moves Workers. |
| 24 | ingress_port | text | the source port the envelope entered through: an App input when key 27 is absent, otherwise the named Connector output; **omitted for the conventional `in`** (absent = omitted), so pre-multiport records remain byte-identical (D-2026-08-08-2) |
| 25 | routes | array | target admissions made before the inbound commit, each `[edge (text), target_instance_ref]`, sorted by canonical `(edge, target_instance_ref)`; omitted only when this ingress produces no routed delivery. The destination reservation returns the generation, so delayed transport cannot enter a successor. The entering envelope is implicit in the INBOUND record. `@loop.in` is the reserved internal edge spelling for root admission into a Loop manager; it records selection but is never a spatial plan edge. |
| 26 | session_ingress | array | present only for Session ingress: `[1, session_ref (bytes16)]` opens a session and `[2, session_ref (bytes16)]` admits one message into an already-open session. Omitted for Event and Call facts, preserving every pre-H1 record byte. |
| 27 | source_connector_path | array | lexical UTF-8 path segments of the inbound Connector whose output introduced this root. Omitted exactly for an explicit App-boundary injection. Derived from the proposal's single `InboundEnvelopeProvenance` authority at acceptance; its presence and structural spelling are the replay-routing fact. |
| 28 | inbound_call | array | present exactly when this Connector-origin root opens an inbound Call: `[inbound_call_ref (bytes16), source_instance_ref]`. The reference is the root id behind a direction-specific type; the instance is selected and reserved before this same INBOUND commit. Omitted for Event, Session and App-boundary facts. |
| 57 | instance_commands | map | Sparse exact-instance closure production, as defined below. Omitted without commands; never consumption on INBOUND. |

**TRANSACTION** (kind 2 — one Executor/manager firing or one controller-owned
Node lifecycle fact;
the commit packet of
execution-model §1) — keys `30–39`, `48–52`, `54–55` and sparse `57`:

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 30 | owner_path | text | the firing Executor/manager or lifecycle owner, root-relative. Graph `l:start`/`l:stop` use the Graph owner path, including empty text for the root App Graph; this does not make Graph an application executor. |
| 31 | instance_ref | array | `[instance_address (bytes), generation (uint)]`, mandatory for every firing; generation is the zero-based journal-derived `l:start` ordinal for that address |
| 32 | trigger | text | namespaced `p:<port>` / `l:<lifecycle>` (envelope.md §3 ; closed `l:` registry) |
| 33 | consumed | array | the consumed input **positions** — each a fixed tuple `[edge (text), source_instance_ref, seq (uint)]` (the cursor key, delivery-semantics §2 — per `(graph-edge × source instance reference)`, NOT per port: several edges feed one port, so `(port, seq)` is not injective); **array sorted by canonical `(edge, source_instance_ref, seq)`** |
| 34 | join_inputs | array | for a join trigger, the presence-completing set (envelope.md §3), each a spine-`id` (bytes 16), **in the handler's declared port order**; this array is deliberately not lexicographically re-sorted, because tuple order is identity; omitted otherwise |
| 35 | state_delta | array | canonical **sorted pair list** `[field_path (text), tagged_value]` of the firing Node's private `var` changes `state → state'` (sub-form below; fields sorted by `field_path`); omitted when empty |
| 36 | emissions | array | **in text/`emission_index` order** (execution-model §4) ; each entry a fixed tuple `[port (text), emission_index (uint), envelope (spine 1–9)]` |
| 37 | timers | array | dated obligations armed by a transaction or accepting inbound fact — each a fixed tuple `[kind (uint), ref (bytes)]`, sorted by `(kind, ref)`. The extend-only kinds are `1 = call timeout`, `2 = join within`, `3 = instance lifetime`, `4 = Saga timeout`, `5 = Loop delay`, `6 = instance drain`, `7 = instance settlement`, `8 = inbound Call request timeout`, `9 = outbound Session handshake timeout`. Kind 3 uses domain `0x21`; kind 5 uses the manager-produced `l:next` envelope id; kinds 6/7 use domain `0x22` plus their kind; kind 8 uses the already-derived `InboundCallRef`; kind 9 uses the candidate `SessionRef`. The due instant is NOT sealed (a persisted, replayable scheduler annotation). |
| 38 | routes | array | target admissions made before commit, each `[delivery_id (bytes16), edge (text), target_instance_ref]`, sorted by canonical tuple; omitted only when the transaction creates no routed delivery. This is the recovery authority for a delivery not yet consumed by its target; replay never re-evaluates `instance.key` and never substitutes a later generation. |
| 39 | cause_id | bytes(16) | causal envelope of a lifecycle transaction. Normally key 33 identifies the same cause and this field is omitted; `l:start`, `l:stop` and later `l:evict` have no consumed cursor, while a graph-response `l:reply` deliberately carries both: key 33 names the response delivery and key 39 preserves the distinct opening root as causal parent. This keeps lifecycle causality autonomous rather than relying on record order. |
| 48 | parent_instance_ref | array | present exactly on `l:start` for a non-root owner; the fixed `[instance_address, generation]` of the enclosing Graph instance. It opens the replayable parent/child lifecycle obligation and is omitted only for the root App owner. A logical child address contains ancestor key/time frames but not ancestor generations, so this fact is not derivable from key 31. |
| 49 | close_scope_ref | array | present exactly on the transaction whose close request wins (`l:closing`, or a handler transaction containing bare `stop`); the fixed instance reference of the root of the Closing scope. Descendant work may drain under this reference, and later external admission reserves its successor. Omitted on ordinary firings. |
| 50 | session_exit_ref | bytes(16) | present exactly on the Connector transaction that terminates a Session. That transaction emits exactly one conventional `exit`, last; its envelope projects the same reference as `meta.ext.session_id`. The fold refuses a second exit, an exit with an open write, or any later message/write. |
| 52 | inbound_call_exit_ref | bytes(16) | present exactly on the Connector `l:reply` transaction that terminates the inbound Call opened by INBOUND key 28. Its cause is the root request; fixed ack consumes zero positions, while a graph response consumes exactly its one real delivery position. Owner/instance equal the opening source and exactly one conventional `exit` is last. The fold refuses an unmatched trigger or any second terminal. |
| 53 | retired loop_fact | array | Historical r1 form, never emitted or reinterpreted by v16. |
| 54 | logical_time | int | the signed nanosecond logical instant since the Unix epoch, present exactly when a checked firing handler can observe `now()`; fixed across guard selection and body evaluation, omitted otherwise. This is an explicit transaction input, not a scheduler due date or host `committed_at`. |
| 55 | loop_protocol | map | Sparse r2 packet: 1 context, 2 routed contexts, 3 produced commands, 4 consumed commands, 5 manager facts. Exact forms below; absent for unrelated transactions. |
| 57 | instance_commands | map | Production or winning target-owned consumption; exact shape below. Absent without commands; separate from Loop r2. |

**Instance closure packet — key 57.** This is causal, unlike scheduler metadata
at key 56. Exactly one field: `1: [command+]` for production, or
`2: command_id` for consumption. Production rows are strictly ID-sorted:
`[command_id, source_kind, source_port, source_instance_ref, scope_instance_ref,
target_instance_ref, cause_id]`, with source kind 1 Graph / 2 Node. IDs are
bytes16; refs use the existing `[address_bytes, generation]` framing. Empty,
mixed, repeated or identity-inconsistent packets refuse. The prefix proves
the exact parent-generation binding; the plan independently proves routing.
Consumption follows pending production order, including across distinct targets.
A prior closure settles redundant commands without an added record. Domain and
version allocations are in §1.3; existing records omit the key entirely.

**Loop r2 packet — key 55.** Empty subfields are omitted. Contexts are
ordered outer-to-inner; outgoing contexts are sorted by delivery reference.
Commands and facts retain their validated causal order. The journal binds
these claims to exact committed admissions and independently checks the fold.

```text
packet := {1: [run_ref*]?, 2: [[delivery_id, [run_ref+]]*]?,
           3: [command_produced*]?, 4: [command_ref*]?, 5: [fact*]?}
command_produced := [command_ref, run_ref, iteration, command_value]
command_value := [1, state_spine]                  // Next
               | [2, cause_id]                    // Again
               | [3, [cause_id, done_kind]]        // Done
done_kind := 1 natural | 2 explicit
fact := [1, run_ref, instance_ref, opening, entry, fuel, state_spine]
      | [2, run_ref, from_iteration, to_iteration, remaining, command_ref, state_spine]
      | [3, run_ref, iteration, admission_id, admitted_spine]
      | [4, run_ref, iteration, close_reason]
      | [5, run_ref, iteration, command_ref, last_state_spine, expired_spine]
opening := [1, inbound_id, entry_port] | [2, delivery_id]
close_reason := 1 natural | 2 explicit_done | 3 stopped
```

Fact tags are Open/Advance/Admit/Close/Expired. Retained Advance also owns its
optional due instant, excluded from this causal form and checked against key 56.
Domains are run `0x14`, command `0x15` and re-admission `0x06`; exact opening
admission, not only business origin, identifies a run. See [Loop](loop.md).

**Retained scheduler metadata — not part of the causal seal**
(D-2026-09-07-2):

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 56 | timer_schedule | array | `[armed_at_ns, [[kind, reference_bytes16, due_at_ns], ...]]`, retained atomically beside the accepting INBOUND or TRANSACTION. Instants are signed i64 logical nanoseconds; due ≥ armed-at. Kinds use the existing 1–9 registry (4 remains reserved). Arms retain actual order, including ordered renewals, never sort by reference. Empty schedules are omitted. |

Storage integrity must cover this required recovery evidence independently of
the causal seal. Key 37 keeps its existing sealed timer references; key 54
keeps its observable `now()` meaning. Key 56 changes neither graph identity,
package bytes nor causal projection. A causal trace alone is not a complete
recovery backup. The field codec and every admitted timer producer retain original dates;
recovery restores Calls, joins, lifetimes, Closing and Loop wakes from accepting
prefixes. Missing dates never default to restart time. Storage is still an
in-memory implementation; this is not a durable-backend claim.

**OUTBOUND / Call** (kind 3 — one external call's WAL record ;
delivery-semantics §4) — keys `40–47` :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 40 | emitting_id | bytes(16) | the id of the envelope that opened the call (the cause) |
| 41 | connector_path | text | the calling node |
| 42 | call_key | bytes | the `idempotency_key` (E-COMPENSATE-KEY) ; omitted only for `compensable` without key |
| 43 | request_digest | bytes(32) | the boundary integrity seal (delivery-semantics §4.2) |
| 44 | status | uint | the SEALED causal states : `1 = open · 2 = closed · 4 = abandoned` ; value `3` is reserved — `discarded` (a late reply after closure) is a NON-causal observability record, never a sealed status |
| 45 | closure | array | present iff status ∈ {closed, abandoned} ; EXACT byteshape fixed BY `status` (`[1,outputs]` response · `[2]` timeout · `[]` abandoned ; sub-forms below), so `status` and `closure` never disagree |
| 46 | edge_cursor | array | present only on an inter-shard delivery — the DURABLE LOGICAL cursor `[edge (text), source_instance_ref, seq (uint)]` (seq never restarts on leader change within that source generation; the transport `leader_epoch` fence is separate and NOT sealed); a transport cursor, NOT an envelope id, outside every id domain |
| 47 | edge | text | the canonical edge of the causing delivery ; with key 40 (`emitting_id`) it reconstructs `delivery_ref` and hence `call_ref` (envelope.md §2) — the record stays autonomous |

**OUTBOUND / Session** (kind 3 — opening and message-write WAL facts;
delivery-semantics §4.5) — key `51` only. The value is one closed array form:

| Tag | Fact | Exact array |
|---|---|---|
| 1 | opening intent | `[1, session_ref (bytes16), emitting_id (bytes16), connector_path (text), request_digest (bytes32)]` |
| 2 | opening confirmed | `[2, session_ref (bytes16), connector_path (text)]` |
| 3 | message-write intent | `[3, session_ref (bytes16), write_ref (bytes16), emitting_id (bytes16), connector_path (text), message_digest (bytes32)]` |
| 4 | message-write settled | `[4, session_ref (bytes16), write_ref (bytes16), connector_path (text)]` |
| 5 | message-write abandoned | `[5, session_ref (bytes16), write_ref (bytes16), connector_path (text)]` |

Call and Session subforms are distinguishable without inspecting a contract:
a Call record carries keys 40–47 and never key 51; a Session record carries
key 51 and none of keys 40–47. An inbound Session terminal is represented by
the ordinary Connector TRANSACTION key 50. An inbound Call uses INBOUND key
28 plus ordinary Connector TRANSACTION key 52 and **never** an OUTBOUND Call
record. Neither exchange adds a fourth record kind.

Key 44's `status` is the internal state of the OUTBOUND WAL record, not a
graph port and not a payload union. M1.3's conventional `exit` port therefore
does not rename this concept by implication. B7 changed the closure payload to
an ordered multi-output completion; B8 changed only the plan package's logical
type transport to v7.

**Normative sub-form encodings.** Every
compound value in the tables above has a fixed shape and, where it is
a tagged union, a **numeric discriminant** — so two implementations
cannot diverge on bytes for the same journal :

- **`tier2` (spine key 7)** = the fixed birth-stamp fields
  (envelope.md §1.2), each present only when born in scope, encoded as
  a canonical sorted list of pairs `[stamp_kind (uint), value]` sorted
  by `stamp_kind`, from the EXACT set :
  `1 = saga_id (bytes 16) · 2 = saga_stack (array of bytes 16) ·
  3 = reserved former loop_id (bytes 16) ·
  4 = reserved former loop_stack (array of bytes 16) ·
  5 = reserved former iter_count (uint)` (extend-only; old bytes remain
  decodable, but D-2026-08-28-1 emits none of tags 3–5 and exposes no source
  binding for them). Loop run identity lives in its journal fold instead of
  every born envelope.
- **The `type_tag` registry** — the fixed value-type tree of
  grammar.md `TYPE_KW` / WHITEPAPER §2.1, published in full,
  extend-only, never renumbered. A `tagged_value` is a 2-element
  array `[type_tag (uint), value]` ; each tag's `value` has ONE pinned
  CBOR encoding so two implementations cannot diverge on bytes :

  | Tag | Type | `value` CBOR encoding (canonical) |
  |---|---|---|
  | 1 | bool | CBOR bool |
  | 2 | int | CBOR integer (32-bit two's-complement range) |
  | 3 | long | CBOR integer (64-bit two's-complement range) |
  | 4 | float | IEEE-754 binary32, FIXED width (canonical NaN, −0.0 kept) |
  | 5 | double | IEEE-754 binary64, FIXED width (canonical NaN, −0.0 kept) |
  | 6 | number | the dynamic numeric of `variant` — a DOCUMENT-origin number is canonicalized by the READER's FIXED rule (revised by D-2026-08-10-7) : normalize the decimal (trailing zeros stripped) ; **`−0` → integer zero** ; a scale-0 value in the signed 64-bit range → the INTEGER domain at honest width (`int` (2) within 32 bits, `long` (3) beyond — the `int_tagged` rule) ; everything else → a DYNAMIC `decimal` (10) at MINIMAL scale, within the decimal128 budget (38 significant digits) ; beyond the budget → TYPE ERROR, never a silent narrowing. A `decimal("…")` CONSTRUCTOR value never passes through this rule — it self-carries its DECLARED scale (D-2026-08-13-1), only the written-scale integer fold applying. A `double` (5) arises only from explicit narrowing (`as double`), never from parsing ; a TYPED `−0.0` double keeps its sign (tags 4/5, unchanged). Never encoded bare |
  | 7 | bytes | CBOR byte string |
  | 8 | text | CBOR text string (UTF-8) |
  | 9 | null | CBOR null |
  | 10 | decimal | `[unscaled (bytes, big-endian two's-complement, MINIMAL — no redundant sign byte ; zero = a single `0x00`), scale (INT, signed)]` ; ONE seal rule — the value seals at the scale it CARRIES — fed by three disjoint origins : a TYPED `decimal(p, s)` carries its **DECLARED scale** — never re-minimized, so `decimal(p,2)` value `1` is always `[100, 2]` ; a DOCUMENT-origin dynamic (a `variant` number, tag-6 reader rule) carries its **MINIMAL scale** (trailing zeros stripped) ; a `decimal("…")` CONSTRUCTOR dynamic carries its **DECLARED scale** (D-2026-08-13-1 — `decimal("1.50")` seals `[150, 2]`). The DYNAMIC origins stay disjoint from the integer domain by the WRITTEN-SCALE FOLD — both collapse a scale-≤-0 value representable in i64 to the integer tags, so no dynamic decimal ever hides an integer ; a TYPED decimal needs no fold, its declared type carrying the context. Budget, exact : `unscaled` in the signed 128-bit range with at most 38 significant decimal digits, `scale` ∈ [−6143, +6144] (the IEEE decimal128 exponent domain, Arrow-anchored) ; outside either bound → refusal, never a narrowing (D-2026-08-10-7) |
  | 11 | date | CBOR integer — days since 1970-01-01 |
  | 12 | timestamp | CBOR integer — **i64** NANOSECONDS since epoch (Arrow-anchored ; range ≈ 1677–2262 ; out of range = TYPE ERROR, never a bignum ; timezone semantics live in the resolved `DataType`) |
  | 13 | timestamp_ntz | CBOR integer — **i64** NANOSECONDS in the timezone-naive timeline (same bound) |
  | 14 | time | CBOR integer — **i64** NANOSECONDS since midnight |
  | 15 | uuid | CBOR byte string (16) |
  | 16 | duration | CBOR integer — NANOSECONDS (`duration("1ns")` is exact ; i64 ns ≈ ±292 y) |
  | 17 | struct | canonical sorted `[field (text), tagged_value]` pairs |
  | 18 | map | canonical sorted `[tagged_value (key), tagged_value]` pairs ; keys are compared AFTER canonicalization, and **two keys that canonicalize equal (e.g. distinct NaN payloads → the one canonical NaN) are rejected, never a silent collision** — a STATIC literal → the `E-DUP-KEY` diagnostic (diagnostics.md), a RUNTIME-built map → a typed language error |
  | 19 | array | ordered list of `tagged_value` (`table` canonicalizes here, as its `array<struct>` view — not a distinct tag) |
  | 20 | enum | the closed symbol as `text` (schema-order-independent) |
  | 21 | union | `[branch_index (uint), tagged_value]` — the branch's 0-based position in the union's declared order. **Branches MUST be pairwise disjoint : overlapping/identical branches (`int \| int`, `int \| long`, `number \| long`) are a compile error `E-UNION-OVERLAP`, so a value maps to exactly one `branch_index`** |
  | 22 | variant | the dynamic tree, encoded as the `tagged_value` of its resolved concrete tag |
  | 23 | uint | CBOR unsigned integer (u64 range) — the Rastok `uint` domain (originally introduced by the CEL-compatible evaluator) ; appended 2026-08-07 (see errata E-2026-08-07-1) : it previously borrowed tag 3, making `long 2^31` and `uint 2^31` seal identically |
  | 24 | interval | `[months (int), days (int), nanoseconds (int)]` ; the resolved interval `DataType` fixes the admitted component shape (`year_month`, `day_time`, `month_day_nano`, or Avro's unsigned month/day/millisecond profile) |

  The historical v6 row for tag 21 described only structural
  `[branch_index, tagged_value]`. The M1.3 logical value algebra also has
  name-stable tagged unions; its canonical projection is
  `[case_name (text), tagged struct payload]`, deliberately distinguishable
  from the unsigned structural selector. V7 freezes both selector forms as
  part of one registry rather than retroactively extending v6.

  The materialized carrier does not duplicate its resolved type parameters.
  Integer/float width, fixed length, decimal precision, temporal unit and
  timestamp timezone come from the plan/schema selected by the record's
  `graph_version`; the value carries only the canonical data and semantic
  discriminants. V7 therefore seals a typed value from `(resolved DataType,
  Value)` while storing that schema context once in the retained plan/manifest,
  not as a full type tree in every envelope. The old `value_tagged` rows remain
  byte-stable; v7 activates the complete family without renumbering them.

  (Sort keys — struct by `field` UTF-8 ; map by the key's canonical
  `tagged_value` bytes. All temporal logical types share the Arrow
  nanosecond convention — no µs anywhere in the seal.)
- **`instance_address`** is the canonical CBOR
  bytes of `[owner_path (array<text segment>), frames (array)]`. Segment
  boundaries are identity: no dotted display rendering is ever sealed.
  `frames` is ordered from
  outermost to innermost owner. Each frame is
  `[frame_owner_path (array<text segment>), frame_kind (uint), payload]`, with the
  extend-only kinds `1 = selected_key` (payload = `[resolved_key_type,
  tagged_value]`, using the plan's canonical `DataType` sub-form and the value
  registry above) and `2 = processing_time_box` (payload = the epoch-aligned
  box-start timestamp in signed nanoseconds). The type is required because a
  materialized `Value` deliberately does not duplicate integer width, temporal
  unit, decimal precision, enum domain, or struct field types. When one owner contributes both
  frames, selected-key precedes time-box. Singleton ownership contributes no
  frame; the static `owner_path` still distinguishes Nodes. Generation,
  virtual shard, Worker, Host, lease, and leader epoch are never address
  fields. An **`instance_ref`** is the fixed array `[instance_address (bytes),
  generation (uint)]`. Generation is the zero-based ordinal of `l:start` for
  that address; the target reserves it before producer commit and the
  corresponding `l:start` record carries it in key 31. Instance refs, not bare
  addresses, identify live mailbox/state/lifecycle ownership and occur in
  record keys 25/31/33/38/46/48/49. The address bytes alone remain the stable
  shard hash input; generation changes neither placement nor call identity.
- **The `encoding_tag` registry** — EXACTLY the fixed internal
  representations of envelope.md §5, `raw | avro | arrow-ipc`,
  extend-only, never renumbered : `1 = raw (identity bytes, no codec)
  · 2 = avro · 3 = arrow-ipc`. **A JSON payload is `[raw, bytes]` with
  `meta.ext.content_type = media_type("application/json")`, NOT a separate tag** (the specification
  excludes a JSON tag) ; `json / protobuf / parquet / msgpack` are
  *boundary* codecs a Connector speaks on the wire, never internal
  `data` representations — so two runtimes cannot seal the same JSON as
  `[raw,…]` vs `[json,…]`. `data` (spine key 8) is
  `[encoding_tag (uint), bytes]` — the bytes are opaque, compared
  as-is, never re-encoded.
- **`state_delta` (TX key 35)** = a canonical sorted list of pairs
  `[field_path (text), tagged_value]` sorted by `field_path` — every value
  belongs to the firing Node instance at key 31 and is self-describing via the
  `type_tag` registry above. No owner qualifier is needed because mutable
  state cannot cross a Node boundary.
- **Historical v15 `loop_fact` (TX key 53)** is the sole r1 run-state
  authority for v15 packages. A
  `LoopRunRef` is `derive(0x13, instance_address ‖ generation_u64_le ‖
  external_root_id)`. An immediate re-entry envelope id is
  `derive(0x05, loop_run_ref ‖ iteration_u64_le ‖ selected_next_id)`; its
  parent is the selected `next` emission and its origin remains the external
  root. The selected emission stays in key 36, while key 53 carries the
  manager-owned re-entry or exact `LoopExpired` output. Extend-only fact tag 5
  records `admit(run_ref, iteration)`, the exactly-once `l:next` crossing after
  route selection and bounded-capacity proof. A replay folds these facts; no
  mutable counter or arrival order is sealed. A delayed re-entry retains its
  scheduler due instant outside the canonical key-53 array and seals
  `(5, reentry_envelope_id)` in key 37, so recovery can re-arm the same
  obligation without changing causal identity. V16 does not extend or reuse
  this shape; the current r2 form uses key 55 below.
- **`app_meta` values (spine key 9)** : each triple's third element is
  a `tagged_value` (per the registry above).
- **`timers` (TX key 37)** = tuples `[timer_kind (uint), ref (bytes 16)]`,
  `timer_kind` from `1 = call_timeout · 2 = join_within ·
  `3 = instance_lifetime · 4 = saga_timeout · 5 = loop_delay ·
  6 = instance_drain · 7 = instance_settlement ·
  8 = inbound_call_timeout · 9 = session_handshake`
  (extend-only), `ref` = the obligation's derived 16-byte identity —
  for a call budget, the `delivery_ref` of the arming delivery
  (envelope.md §2); kind 3 derives under domain `0x21` from
  `(instance_address, generation)`, kind 5 reuses the exact `l:next` envelope
  id, while kinds 6/7 derive under domain
  `0x22` from `(instance_address, generation, timer_kind)`. The signed
  nanosecond due instant is never sealed (annotation).
- **`session_ref` and `session_write_ref`** use the same injective framed
  derivation as every other identity in this registry: one domain byte, then
  for each listed component `u32_le(length) ‖ bytes`. Owner path segments are
  separate fields, never a dotted display string.
  - inbound opening: `derive(0x11, 0x01 ‖ connector_path_segment* ‖ opening_root_id)`;
  - outbound opening: `derive(0x11, 0x02 ‖ connector_path_segment* ‖ emitting_id ‖ request_digest)`;
  - message write: `derive(0x12, session_ref ‖ emitting_id ‖ message_digest)`.
  `SessionRef` and `SessionWriteRef` are opaque 16-byte logical references.
  Socket, address, negotiated protocol, Host, Worker, placement and arrival
  order never enter these recipes. Their lowercase hexadecimal text spelling
  is the sole `meta.ext.session_id` projection; causal record fields always
  carry the raw 16 bytes.
- **`closure` (OUTBOUND key 45) — the EXACT byteshapes.**
  The rule is « one call = exactly one closure ». The SEALED
  causal `status` (key 44) is exactly `1 open · 2 closed · 4 abandoned`
  (value `3 discarded` is NOT a sealed status — see below). Key 45 is
  present **iff `status ∈ {closed, abandoned}`**, and its byteshape is
  fixed :
  - `status = 1 open` → key 45 **absent**.
  - `status = 2 closed`, `closure = response` → `[1, outputs]` (a
    2-element array). `outputs` is an ordered array of
    `[port_text, data_form]` drafts. It contains exactly one `exit`, last;
    no draft has an envelope id yet because the VM mints every envelope in
    the following atomic connector TRANSACTION.
  - `status = 2 closed`, `closure = timeout` → `[2]` (a **1-element**
    array — no second element, never `[2, null]` ; `l:timeout` emits
    `TimeoutExpired` as its own TRANSACTION).
  - `status = 4 abandoned` → `[]` (the **empty** array — present but
    zero-length ; recovery triage or bounded Closing resolved
    `OutcomeUnknown`, whose envelope is an `l:recover` TRANSACTION emission).
    These are the two supervision paths that may mint that declared case.
  - `discarded` (a LATE response arriving AFTER closure —
    delivery-semantics §4 / execution-model §6.3, **graph-invisible**) is
    **NOT a sealed OUTBOUND status** : it is a separate
    NON-CAUSAL observability record, never part of the seal. Its payload
    (the late completion) is a diagnostic
    annotation, so two runs seal identically regardless of a discarded
    late reply. The correlator (runtime) decides it; the adapter never
    returns it as a normal completion.
- **`consumed` / `join_inputs` / `emissions`** — the fixed tuples and
  imposed array orders stated in their rows above.

**Run-affecting bindings are attested.** The contract
`run = f(graph, journal)` requires that any
deployment binding able to change the run — a `var` default, a
`timeout`/`lifetime`, a partner URL, the tzdata version, a class
policy — be fixed in an **immutable run-header attestation** (its version
or content hash) before first use. This is journal-owned control metadata,
not another App causal record species. Recovery under a differing
attestation is a typed refusal, the `E-REPLAY-VERSION-MISMATCH`
family. Secrets stay out of the journal in clear ; only their
logical/provider coordinates and version are attested.

**Collision guarantee.** Two records
differ in bytes **iff** they differ in any causal fact : the kind,
any referenced envelope's spine (incl. the namespaced `app_meta`
triples and `tier2` stamps), the INBOUND `(connector_path, replica,
inbound_seq)` root recipe, the firing node/key/trigger, the consumed
positions, the typed `state_delta`, the ordered emissions, the call
key/digest/status and the discriminated `closure`, or the Session opening,
message-write, settlement, abandonment and terminal reference. **Scheduler due
time is NOT among them** (it is derivable — an annotation, above); an explicit
`now()` observation is the narrow key-54 exception:
two runs differ in the seal because their *records and orders*
differ, or because an expression received a different sealed time input.
Conformance vectors (a corpus of
minimally-differing record pairs proven to differ in bytes) are required for
the journal/golden implementation.

**Key table — segment header** : `1 = seq (uint)`,
`2 = semantic_hash (bytes(32))`. Extend-only, same rule.

**Causal core vs annotation ring.** Criterion : *can two correct
executions differ on this field ?* Yes → annotation. No → core.

| Causal core (in the canonical form, compared bytewise) | Annotations (recorded if useful, excluded from the form and the seal) |
|---|---|
| record kind ; full `meta.sys` (id, parent_id, origin_id, role, graph_version) ; port ; `emission_index` ; the causal record fields (consumed set, state delta, ordered emissions, call/session identities and transitions) ; every typed port payload ; explicit transaction `logical_time` when `now()` is observable | scheduler arming/due time (derivable — annotation, simulation-semantics §6.1) ; wall-clock `committed_at` ; execution durations ; cost / metrics ; host info ; runtime version ; thread ids ; socket ids ; negotiated protocol ; all non-normative diagnostics |

**M1.8 A2 checkpoint.** Source data annotations are different from the record
annotation ring: their canonical resolved side-table is semantic identity
because it changes handling and accounting policy. A2 transports that table
categorically in package v15 without adding a `DataType` tag. Concrete
secret values, deployment bindings, cost profiles and exporter configuration
remain outside graph identity; run-affecting versions/digests use the boot
attestation rule above. Typed measure/cost samples remain in the non-causal
ring, and no existing record moves merely because an annotation vocabulary is
recognized.

**M1.8 A4 deployment checkpoint.** Exact `env()` / `secret()` source
references hash as their existing categorical expression paths and selected
boot site. A physical environment value, bearer material, provider coordinate,
privacy-profile fact or `--deployment` path does not hash. The HttpClient
prepared descriptor seals a logical `SecretRef`; a rotated material value
therefore keeps graph and Call-request identity stable, while recovery under a
different environment-value digest or provider coordinate/version refuses
because the immutable run header differs. The same header stores the stable
privacy-profile reference and digest before personal data is admitted. It is
control metadata outside the causal record array. A4 allocates no package
version, semantic extension, record key or value/type tag; every earlier
package, witness and trace remains byte-identical.

**M1.8 A7 accounting checkpoint.** The optional deployment cost profile is
fixed in the same immutable run header as `(stable profile reference,
canonical digest)`. Price rows, provider/model dimensions and calculated cost
remain outside graph/package and causal-record identity. Replaying with a
supplied profile whose reference or digest differs from the header refuses;
omitting the profile keeps exact usage and yields typed `unpriced` remainders.
Accounting observations are derived lazily from sealed envelopes plus the
validated annotation table, so A7 adds no record key and moves no existing
package, semantic witness or causal trace.
