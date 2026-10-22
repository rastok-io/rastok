# Identity and canonical form

> **Format 21, diagnostic recording 4.** The [Node realignment inventory](../architecture/node-realignment-identity.md)
> records the package-only representation change; semantic hashes, journal
> records and causal traces are unchanged.
> Only this format is readable. The registry is now extend-only: no
> renumbering or reuse of an allocated key, tag or domain.
> Changing the public version label to 1 remains a pre-publication TODO.

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

Physical Driver implementation names and per-Node deployment selection are
also outside the semantic hash and journal identities. They choose an
implementation of the already-resolved Contract, not another Contract or
execution model. Equal causal inputs and outcomes keep equal identities;
this is not a claim that different external providers return equal outcomes.

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
frontend/compiler/lowering profile, ABI and `GuestRunner` identities.

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

### 1.1 Semantic structures and transport

A Connector's identity includes its exact `ContractId`, required roles,
exchange shape, complete resolved port types and effective attributes.
`behavior` and the boolean `idempotent` promise are independent.
Physical Drivers, their libraries and negotiated transport state do not salt
the graph hash.

**One publisher namespace rule.** Every qualified `namespace/name@version`
identity defined by the standard library uses `std/`, including Contracts,
decoders, protected annotation definitions and the environment secret provider
`std/env@1`. Every other publisher uses a dotted namespace, including test
fixtures (`test.local/test@1`) and deployment profiles
(`privacy.example/support@1`, `pricing.example/model-api@2026-08-30`).
The rule is independent of whether the reference enters a plan hash or only
a run-header attestation. It governs the publisher, not the version syntax:
Contracts and decoders require a canonical positive major, while profile
versions remain their existing opaque immutable references. Logical secret
paths, environment names and unqualified policy names are not publisher ids.

The canonical HIR carries lexical Graph ownership, instance selection,
lifetimes, bounded Closing and phased immutable expressions. A Graph is a
scope, never an application handler owner. Nested Graph ports may be flattened
spatially, but their ownership and admission policy cannot disappear.

Closed Dispatch arms and routing cases are canonical sets; open Dispatch arms,
join inputs and ordered emissions retain source order. A single-port `where`
and its equivalent open match have the same HIR. A join's admission guard is
a distinct fact: false preserves candidates.

Loop and Retry keep their separate checked contracts. Loop entry, terminals,
exhaustion target/type, fuel and proof are transported and independently
validated; Retry retains attempts, cases, delay and the original request's
admission identity. Manager commands and exact-instance stop commands are not
spatial edges. The active HIR stream domains include `rastok.loop.r2`,
`rastok.retry.r1`, `rastok.dispatch.r1`, `rastok.routing.r1`,
`rastok.graph.owners.r1`, `rastok.instances.r1` and `rastok.closing.r1`.
A domain's textual suffix is part of its current identity, not an alternate
package reader.

Annotations travel as typed policy side tables, not new logical value tags.
The protected numerical binding is `std/measure@3`, with exactly
`{ id: text, unit: text }`. Every actual measured quantity remains in the
original typed envelope, including frames held by Retry. Missing quantities
are absent, never invented zeroes. M1 has no pricing or aggregation report.

The [plan package](../architecture/plan-package.md) transports these facts
and an asserted HIR hash under an integrity checksum. The package version,
physical layout and checksum are not additional semantic-hash inputs.
Hostile decoding re-derives the executable claims without a ContractCatalog.

The package contains one Node table: Graph, Operator or Connector, with the
App Graph at index zero. Each Graph owns its reference wiring; each Operator
owns its handlers. `NodeIndex` and execution-local delivery indexes never
enter the identity recipes below. Paths and exact instance generations remain
their inputs. Derived execution indexes are not separate transported claims.

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

**M1 boundary.** The journal is in memory. Diagnostic recordings keep the
validated plan and immutable run header; replay verifies their association.
There is no durable manifest service or on-disk WAL in M1.

**Durable deployment design, M2.** A manifest is an append-only authority
independent of prunable journal segments. It maps each deployment number to
the semantic hash and the schemas needed for its records. Segment headers
cross-check that mapping; they do not replace it. Recovery with another graph
or run-affecting attestation refuses rather than silently migrating records.
An application-level schema conversion is an explicit typed Operator emitting
a new envelope with normal lineage, not a compatibility reader.

## 3. The canonical record form

The journal contains immutable records with **sealed causal fields** and
**unsealed recovery fields**. Both are kept at commit; unsealed does not mean
mutable or optional. Deadlines and winning Call closure instants must survive
recovery exactly, but do not enter a causal trace seal.

The complete record codec and the canonical causal projection are different
formats. A diagnostic recording includes the former. A golden compares the
latter. A causal trace alone is therefore not a recovery backup.

**Encoding : deterministic CBOR — RFC 8949 §4.2, cited in full** :
shortest-form integers, definite lengths only, and the fixed unsigned
keys of record maps emitted in ascending table order (which is their
canonical encoded-key order). Pair-list sub-forms name their own
order: tag-15 struct fields and state-delta paths use UTF-8 name order;
tag-16 Rastok maps use each key's canonical `tagged_value` bytes;
metadata key 6 uses UTF-8 `(zone, field)` order.
**Floats are the exception to shortest-form : a value's
materialized floating carrier is always IEEE-754 binary64, with the logical
width checked by the resolved type — so `1.5` is never silently narrowed to
binary16 by one implementation and not another.** `NaN` has one canonical
quiet representation (`0x7ff8000000000000`);
`-0.0` is preserved distinct from `+0.0`. Record-map keys are **unsigned
integers** from the tables below; since keys are fixed, emission in table order satisfies the
sorting rule at zero runtime cost. An absent field is an **omitted key**,
never a null. The fast seal of a trace is `blake3` over the concatenated
canonical records ; record-by-record comparison exists to *explain* a
failure (`rastok trace dump` renders the form for review — derived,
non-normative).

Record kinds are INGRESS, TRANSACTION and EGRESS; inbound/outbound describe
exchange direction. EGRESS is an effect initiated by the graph. The response
to an inbound Call terminates in a TRANSACTION's exchange-exit field.

**Per-species tables.** A single flat table would conflate the three
record kinds and omit fields that distinguish them : two semantically
different journals could project to the same bytes. The canonical form is a
**shared canonical form + one extension table per kind**, so that *every
causal fact a record carries has a key*. All tables are extend-only
(append, never renumber, never reuse) ; an absent field is an
omitted key.

**The canonical envelope form** (keys `1–6`) — how any envelope referenced by
a record is canonicalized (its full `meta.sys`, per envelope.md §1) :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 1 | id | bytes(16) | blake3 first 128 bits (`uuid128`), per envelope.md §2 |
| 2 | parent_id | bytes(16) | omitted at a root |
| 3 | origin_id | bytes(16) | |
| 4 | graph_version | uint (u64) | constant 1 in embedded/sim (§2) |
| 5 | data | array | the tagged payload `[encoding_tag (uint), bytes]` (sub-form below) — opaque, zero-copy, compared as bytes and never re-encoded; an outcome/exit payload is an ordinary envelope, no separate key |
| 6 | app_meta | array | the causal writable metadata — a **canonical sorted list of triples `[zone (text), field (text), tagged_value]`**, sorted UTF-8 by `(zone, field)` ; `zone` ∈ {`app`, `ext`} (D-2026-08-10-1 — root customs died), `field` is the flat dotted key (`audit.origin`) ; sealed because a handler reads them ; omitted when empty |

**Record — shared header** (key `7`; observed time and unsealed deadlines are distinguished below) :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 7 | kind | uint | 1 = INGRESS · 2 = TRANSACTION · 3 = EGRESS |

**Time has two roles.** A scheduler's exact creation/due instant and a Call's
winning closure instant are unsealed recovery inputs. They are retained, not
recomputed from restart time. Two runs may have different exact arrival times
yet the same causal record sequence. If a timeout wins instead of a response,
that difference changes the record sequence and its seal.

A processing-time box start is part of the selected instance identity, hence
sealed. An explicit handler `now()` observation is also sealed, at key 39:
the logical instant is fixed across predicate and body evaluation. These are
program-visible inputs, not performance measurements.

**INGRESS** (kind 1 — an envelope entered) — keys `8–16`, plus optional deadline ids `24` and instance commands `42`:

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 8 | envelope | map | the canonical form (keys 1–6) of the entering envelope |
| 9 | connector_path | text | the ingress node's root-relative path (root recipe input, envelope.md §2) |
| 10 | inbound_seq | uint | the atomic sequence for `(connector_path, replica)` — assigned by the owning shard's leader ; root recipe input |
| 11 | replica | uint | the logical inbound Connector replica in `IngressProvenance`; sealed so the record autonomously reconstructs `(connector_path, replica, inbound_seq)`. It may be allocated by the leader responsible for that inbound Connector, but is never the target-routing shard and never changes when a destination moves Workers. |
| 12 | ingress_port | text | the source port the envelope entered through: an App input when key 15 is absent, otherwise the named Connector output; **omitted for the conventional `in`** (absent = omitted), the absence denotes that exact conventional port |
| 13 | routes | array | target admissions made before the inbound commit, each `[edge (text), target_instance_generation]`, sorted by canonical `(edge, target_instance_generation)`; omitted only when this ingress produces no routed delivery. The destination reservation returns the generation, so delayed transport cannot enter a successor. The entering envelope is implicit in the INGRESS record. `@loop.in` is the reserved internal edge spelling for root admission into a Loop manager; it records selection but is never a spatial plan edge. |
| 14 | session_ingress | array | present only for Session ingress: `[1, session_id (bytes16)]` opens a session and `[2, session_id (bytes16)]` admits one message into an already-open session. Omitted for Event and Call facts. |
| 15 | source_connector_path | array | lexical UTF-8 path segments of the inbound Connector whose output introduced this root. Omitted exactly for an explicit App-boundary injection. Derived from the proposal's single `IngressProvenance` authority at acceptance; its presence and structural spelling are the replay-routing fact. |
| 16 | inbound_call | array | present exactly when this Connector-origin root opens an inbound Call: `[inbound_call_id (bytes16), source_instance_generation]`. The reference is the root id behind a direction-specific type; the instance is selected and reserved before this same INGRESS commit. Omitted for Event, Session and App-boundary facts. |
| 42 | instance_commands | map | Sparse exact-instance closure production, as defined below. Omitted without commands; never consumption on INGRESS. |

**TRANSACTION** (kind 2 — one Executor/manager firing or one controller-owned
Node lifecycle fact;
the commit packet of
execution-model §1) — keys `17–26`, `34–36`, `38–40`, `42–43`:

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 17 | owner_path | text | the firing Executor/manager or lifecycle owner, root-relative. Graph `l:start`/`l:stop` use the Graph owner path, including empty text for the root App Graph; this does not make Graph an application executor. |
| 18 | instance_generation | array | `[instance_address (bytes), generation (uint)]`, mandatory for every firing; generation is the zero-based journal-derived `l:start` ordinal for that address |
| 19 | trigger | text | namespaced `p:<port>` / `l:<lifecycle>` (envelope.md §3 ; closed `l:` registry) |
| 20 | consumed | array | the consumed input **positions** — each a fixed tuple `[edge (text), source_instance_generation, seq (uint)]` (the cursor key, delivery-semantics §2 — per `(graph-edge × source instance reference)`, NOT per port: several edges feed one port, so `(port, seq)` is not injective); **array sorted by canonical `(edge, source_instance_generation, seq)`** |
| 21 | join_inputs | array | for a join trigger, the presence-completing set (envelope.md §3), each a envelope `id` (bytes 16), **in the handler's declared port order**; this array is deliberately not lexicographically re-sorted, because tuple order is identity; omitted otherwise |
| 22 | state_delta | array | canonical **sorted pair list** `[field_path (text), tagged_value]` of the firing Node's private `var` changes `state → state'` (sub-form below; fields sorted by `field_path`); omitted when empty |
| 23 | emissions | array | **in text/`emission_index` order** (execution-model §4) ; each entry a fixed tuple `[port (text), emission_index (uint), envelope (canonical keys 1–6)]` |
| 24 | deadline_ids | array | dated obligations created by a transaction, accepting INGRESS fact or Retry EGRESS closure — each a fixed tuple `[kind (uint), ref (bytes)]`, sorted by `(kind, ref)`. Kinds are `1 = call timeout`, `2 = join within`, `3 = instance lifetime`, `4 = recurrence delay`, `5 = instance drain`, `6 = instance settlement`, `7 = inbound Call timeout`, `8 = Session handshake`. Kind 3 (InstanceLifetime) is retained at key 41 only and never sealed here; kind 4 names the exact Loop/Retry admission; kinds 5/6 use InstanceClosing plus their kind; kind 7 uses the InboundCallId; kind 8 uses the SessionId. The due instant is NOT sealed (retained separately for recovery). |
| 25 | routes | array | target admissions made before commit, each `[delivery_id (bytes16), edge (text), target_instance_generation]`, sorted by canonical tuple; omitted only when the transaction creates no routed delivery. This is the recovery authority for a delivery not yet consumed by its target; replay never re-evaluates `instance.key` and never substitutes a later generation. |
| 26 | cause_id | bytes(16) | causal envelope of a lifecycle transaction. Normally key 20 identifies the same cause and this field is omitted; `l:start`, `l:stop` and later `l:evict` have no consumed cursor, while a graph-response `l:reply` deliberately carries both: key 20 names the response delivery and key 26 preserves the distinct opening root as causal parent. This keeps lifecycle causality autonomous rather than relying on record order. |
| 34 | parent_instance_generation | array | present exactly on `l:start` for a non-root owner; the fixed `[instance_address, generation]` of the enclosing Graph instance. It opens the replayable parent/child lifecycle obligation and is omitted only for the root App owner. A logical child address contains ancestor key/time segments but not ancestor generations, so this fact is not derivable from key 18. |
| 35 | close_scope_generation | array | present exactly on the transaction whose close request wins (`l:closing`, or a handler transaction containing bare `stop`); the fixed instance reference of the root of the Closing scope. Descendant work may drain under this reference, and later external admission reserves its successor. Omitted on ordinary firings. |
| 36 | session_exit_id | bytes(16) | present exactly on the Connector transaction that terminates a Session. That transaction emits exactly one conventional `exit`, last; its envelope projects the same reference as `meta.ext.session_id`. The fold refuses a second exit, an exit with an open write, or any later message/write. |
| 38 | inbound_call_exit_id | bytes(16) | present exactly on the Connector `l:reply` transaction that terminates the inbound Call opened by INGRESS key 16. Its cause is the root request; fixed ack consumes zero positions, while a graph response consumes exactly its one real delivery position. Owner/instance equal the opening source and exactly one conventional `exit` is last. The fold refuses an unmatched trigger or any second terminal. |
| 39 | logical_time | int | the signed nanosecond logical instant since the Unix epoch, present exactly when a checked firing handler can observe `now()`; fixed across guard selection and body evaluation, omitted otherwise. This is an explicit transaction input, not a scheduler due date or host `committed_at`. |
| 40 | loop_protocol | map | Sparse r2 packet: 1 context, 2 routed contexts, 3 produced commands, 4 consumed commands, 5 manager facts. Exact forms below; absent for unrelated transactions. |
| 42 | instance_commands | map | Production or winning target-owned consumption; exact shape below. Absent without commands; separate from Loop r2. |
| 43 | retry_protocol | map | Sparse Retry packet, also carried on its outbound Call facts. Correlation and Opened/Admitted/Held/Consumed/Advance/Published/Close use the exact forms below, checked against the retained prefix. Absent on unrelated records, INGRESS and Sessions. |

**Instance closure packet — key 42.** This is causal, unlike scheduler metadata
at key 41. Exactly one field: `1: [command+]` for production, or
`2: command_id` for consumption. Production rows are strictly ID-sorted:
`[command_id, source_kind, source_port, source_instance_generation, scope_instance_generation,
target_instance_generation, cause_id]`, with source kind 1 Graph / 2 Node. IDs are
bytes16; refs use the existing `[address_bytes, generation]` framing. Empty,
mixed, repeated or identity-inconsistent packets refuse. The prefix proves
the exact parent-generation binding; the plan independently proves routing.
Consumption follows pending production order, including across distinct targets.
A prior closure settles redundant commands without an added record. The domain is InstanceStop (`0x0e`, envelope.md §2); unrelated records omit the key.

**Loop r2 packet — key 40.** Empty subfields are omitted. Contexts are
ordered outer-to-inner; outgoing contexts are sorted by delivery reference.
Commands and facts retain their validated causal order. The journal binds
these claims to exact committed admissions and independently checks the fold.

```text
packet := {1: [run_id*]?, 2: [[delivery_id, [run_id+]]*]?,
           3: [command_produced*]?, 4: [command_id*]?, 5: [fact*]?}
command_produced := [command_id, run_id, iteration, command_value]
command_value := [1, state_form]                  // Next
               | [2, cause_id]                    // Again
               | [3, [cause_id, done_kind]]        // Done
done_kind := 1 natural | 2 explicit
fact := [1, run_id, instance_generation, opening, entry, fuel, state_form]
      | [2, run_id, from_iteration, to_iteration, remaining, command_id, state_form]
      | [3, run_id, iteration, admission_id, admitted_form]
      | [4, run_id, iteration, close_reason]
      | [5, run_id, iteration, command_id, last_state_form, expired_form]
opening := [1, inbound_id, entry_port] | [2, delivery_id]
close_reason := 1 natural | 2 explicit_done | 3 stopped
```

Fact tags are Open/Advance/Admit/Close/Expired. Advance also retains its
optional due instant, excluded from this causal form and checked against key 41.
Domains are run `0x08`, command `0x09` and re-admission `0x03`; exact opening
admission, not only business origin, identifies a run. See [Loop](loop.md).

**Retry packet — key 43.** Empty subfields are omitted; facts and batch
emission ids retain commit order. The operation names the exact opening
delivery, not a business deduplication key. The original validated plan and
prepared invocation remain pinned for every attempt.

```text
packet := {1: [operation_id, attempt_ordinal]?, 2: [fact+]?}
fact := [1, operation_id, opening_delivery_id, entry, plan_hash, contract_id, pin]
      | [2, operation_id, attempt_ordinal, admitted_form, delivery_id]
      | [3, operation_id, attempt_ordinal, call_id, [emission_id*]] // Held
      | [4, operation_id, attempt_ordinal, call_id, [emission_id*]] // Consumed
      | [5, operation_id, from_ordinal, to_ordinal, call_id]        // Advance
      | [6, operation_id, attempt_ordinal, call_id, [emission_id*]] // Published
      | [7, operation_id, attempt_ordinal, close_reason]
pin := [opening_envelope_id, request_digest, [[name, tagged_value]*], call_key_or_null]
close_reason := 1 final | 2 stopped
```

Fact tags 1 and 2 are Opened and Admitted. Settings are name-sorted; ids are
bytes16, the plan hash and digest bytes32, and names/optional call keys text.
The lossless codec additionally keeps the prepared request bytes and the
invocation's logical-time input; the causal projection above keeps their
explicit pin, not a second encoding of the whole request. A Held batch is
consumed or published exactly once, with precisely the same ordered ids.
The delay's dates belong to keys 41 and 44, not this packet.

**Unsealed scheduler metadata — not part of the causal seal**
(D-2026-09-07-2):

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 41 | deadlines | array | `[created_at_ns, [[kind, reference_bytes16, due_at_ns], ...]]`, retained atomically beside the accepting INGRESS, TRANSACTION or Retry EGRESS closure. Instants are signed i64 logical nanoseconds; due ≥ created-at. The eight admitted kinds are 1 through 8; unknown bytes refuse. Deadlines retain creation order, including ordered renewals, never sort by reference. Empty schedules are omitted. |
| 44 | closed_at | int | Original signed logical nanoseconds of every winning outbound Call closure, including non-Retry Calls. Absent on Open and on INGRESS/TRANSACTION/Session facts. A Retry delay deadline uses this instant as its key-41 `created_at`; recovery never substitutes restart time. |

Storage integrity must cover this required recovery evidence independently of
the causal seal. Key 24 keeps its existing sealed deadline ids; key 39
keeps its observable `now()` meaning. Key 41 changes neither graph identity,
package bytes nor causal projection. A causal trace alone is not a complete
recovery backup. The field codec and every admitted deadline producer retain original dates;
recovery restores Calls, joins, lifetimes, Closing, Loop and Retry wakes from accepting
prefixes. Missing dates never default to restart time. Storage is still an
in-memory implementation; this is not a durable-backend claim.

**EGRESS / Call** (kind 3 — one external call's WAL record ;
delivery-semantics §4) — keys `27–33`, plus sparse `24` and `43` for Retry;
unsealed `41` and `44` are described above:

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 27 | emitting_id | bytes(16) | the id of the envelope that opened the call (the cause) |
| 28 | connector_path | text | the calling node |
| 29 | call_key | bytes | UTF-8 bytes of the optional partner `idempotency_key`; omitted when none is retained. Stable across Retry attempts, but neither its presence nor its value proves idempotence. |
| 30 | request_digest | bytes(32) | the boundary integrity seal (delivery-semantics §4.2) |
| 31 | status | uint | the SEALED causal states : `1 = open · 2 = closed · 3 = abandoned`; a discarded late reply is graph-invisible diagnostic information, not a sealed status |
| 32 | closure | array | present iff status ∈ {closed, abandoned} ; EXACT byteshape fixed BY `status` (`[1,outputs]` response · `[2]` timeout · `[]` abandoned ; sub-forms below), so `status` and `closure` never disagree |
| 33 | edge | text | the canonical edge of the causing delivery ; with key 27 (`emitting_id`) it reconstructs `delivery_id` and hence `call_id` (envelope.md §2) — the record stays autonomous |

Every outbound Call closure also retains **key 44**, `closed_at` as signed
logical nanoseconds; an Open has no value. This field is outside the causal
projection. Retry closures may carry causal deadline ids at key 24,
unsealed schedule key 41 (`created_at = closed_at`) and correlation key 43.
The next-admission reference names the Retry operation/ordinal/preceding Call,
not a Loop command. Recovery never restarts that delay from reboot time.

**EGRESS / Session** (kind 3 — opening and message-write WAL facts;
delivery-semantics §4.5) — key `37` only. The value is one closed array form:

| Tag | Fact | Exact array |
|---|---|---|
| 1 | opening intent | `[1, session_id (bytes16), emitting_id (bytes16), connector_path (text), request_digest (bytes32)]` |
| 2 | opening confirmed | `[2, session_id (bytes16), connector_path (text)]` |
| 3 | message-write intent | `[3, session_id (bytes16), write_id (bytes16), emitting_id (bytes16), connector_path (text), message_digest (bytes32)]` |
| 4 | message-write settled | `[4, session_id (bytes16), write_id (bytes16), connector_path (text)]` |
| 5 | message-write abandoned | `[5, session_id (bytes16), write_id (bytes16), connector_path (text)]` |

Call and Session subforms are distinguishable without inspecting a contract:
a Call record carries keys 27–33 and never key 37; a Session record carries
key 37 and none of keys 27–33. An inbound Session terminal is represented by
the ordinary Connector TRANSACTION key 36. An inbound Call uses INGRESS key
16 plus ordinary Connector TRANSACTION key 38 and **never** an EGRESS Call
record. Neither exchange adds a fourth record kind.

Key 31's `status` is the internal state of an EGRESS record, not an
`exit` port or payload union. A completion contains its ordered output drafts;
the following Connector transaction assigns their envelope identities.

**Normative sub-form encodings.** Every
compound value in the tables above has a fixed shape and, where it is
a tagged union, a **numeric discriminant** — so two implementations
cannot diverge on bytes for the same journal :

- **The value-tag registry.** A `tagged_value` is
  `[type_tag (uint), value]`. Logical types remain a separate registry in
  the plan. A dynamic `variant` uses its concrete value tag, never a wrapper.

  | Tag | Carrier | Canonical value |
  |---|---|---|
  | 1 | bool | CBOR bool |
  | 2 | int | signed integer in i32 range |
  | 3 | long | signed integer outside i32, within i64 |
  | 4 | double | fixed binary64, canonical NaN, signed zero preserved |
  | 5 | bytes | byte string |
  | 6 | text | UTF-8 text |
  | 7 | null | null |
  | 8 | decimal | [minimal signed big-endian unscaled bytes, signed scale] |
  | 9 | date | signed days since epoch |
  | 10 | timestamp | signed i64 nanoseconds since epoch |
  | 11 | timestamp_ntz | signed i64 nanoseconds on the timezone-naive timeline |
  | 12 | time | signed nanoseconds since midnight |
  | 13 | uuid | 16-byte string |
  | 14 | duration | signed i64 nanoseconds |
  | 15 | struct | field-name-sorted [text, tagged_value] pairs |
  | 16 | map | [tagged key, tagged value] pairs sorted by encoded key bytes |
  | 17 | array | ordered tagged values; table uses its array-of-struct form |
  | 18 | enum | symbol text, independent of declaration order |
  | 19 | union | [canonical structural branch index, tagged value] or [case text, tagged struct payload] |
  | 20 | uint | unsigned integer within u64 |
  | 21 | interval | [signed months, signed days, signed nanoseconds] |

  Structural union branches are pairwise disjoint and use canonical type
  order. Tagged-union case names and structural indexes cannot collide.
  Duplicate canonical map keys refuse; reordering struct fields changes no
  canonical value bytes. The resolved type supplies width, fixed size,
  decimal precision, units, timezone and field contracts without duplicating
  a type tree inside every value.

  **Decimal origins stay distinct.** A typed decimal retains its declared
  scale; `decimal(p,2)` value one is `[100, 2]`. A document-origin dynamic
  number strips trailing zeros and folds a scale-zero i64 into int/long.
  A `decimal("1.50")` constructor retains its written scale (`[150,2]`);
  its scale-≤-0 i64 values fold into int/long. Negative document zero folds
  to integer zero, not a guessed double. Typed binary64 negative zero keeps
  its sign. Unscaled values fit signed i128 and 38 significant digits;
  scale is within −6143…6144. Out-of-budget values refuse, never narrow.

  Decoding ccbor DATA checks canonical CBOR, exact byte consumption,
  consistency of the logical value and equality with its canonical
  re-encoding. For example, a long tag containing a small i32 refuses:
  it is not a second representation of that integer.

- **`instance_address`** is the canonical CBOR
  bytes of `[owner_path (array<text segment>), segments (array)]`. Segment
  boundaries are identity: no dotted display rendering is ever sealed.
  `segments` is ordered from
  outermost to innermost owner. Each segment is
  `[segment_scope_path (array<text segment>), segment_kind (uint), payload]`, with the
  extend-only kinds `1 = selected_key` (payload = `[resolved_key_type,
  tagged_value]`, using the plan's canonical `DataType` sub-form and the value
  registry above) and `2 = processing_time_box` (payload = the epoch-aligned
  box-start timestamp in signed nanoseconds). The type is required because a
  materialized `Value` deliberately does not duplicate integer width, temporal
  unit, decimal precision, enum domain, or struct field types. When one owner contributes both
  segments, selected-key precedes time-box. Singleton ownership contributes no
  segment; the static `owner_path` still distinguishes Nodes. Generation,
  virtual shard, Worker, Host, lease, and leader epoch are never address
  fields. An **`instance_generation`** is the fixed array `[instance_address (bytes),
  generation (uint)]`. Generation is the zero-based ordinal of `l:start` for
  that address; the target reserves it before producer commit and the
  corresponding `l:start` record carries it in key 18. Instance refs, not bare
  addresses, identify live mailbox/state/lifecycle ownership and occur in
  record keys 13/18/20/25/34/35. The address bytes alone remain the stable
  shard hash input; generation changes neither placement nor call identity.
- **DATA encoding tags** are `1 = raw`, `2 = ccbor`, `3 = arrow-ipc`.
  DATA (envelope key 5) is `[encoding_tag, bytes]`. External raw bytes are
  not guessed to be JSON, text or canonical values. A reading `bytes` port
  materializes Bytes; a `text` port materializes Text and refuses invalid
  UTF-8. Graph-born values, including text and bytes, use ccbor. Encoded and
  materialized ccbor are representation states of one payload, with the
  same canonical form. A raw byte string equal to those ccbor bytes is a
  different payload because its discriminator differs. Arrow IPC can remain
  opaque. Avro is an import/export schema projection, not internal storage.

- **`state_delta` (TX key 22)** = a canonical sorted list of pairs
  `[field_path (text), tagged_value]` sorted by `field_path` — every value
  belongs to the firing Node instance at key 18 and is self-describing via the
  `type_tag` registry above. No owner qualifier is needed because mutable
  state cannot cross a Node boundary.
- **`app_meta` values (envelope key 6)** : each triple's third element is
  a `tagged_value` (per the registry above).
- **Deadline ids (key 24)** are sorted `[kind, id]` pairs. The eight
  kinds above are a closed typed registry. Call and join ids identify their
  causal obligations; instance lifetime uses domain 12, Closing domain 13
  plus its phase. A recurrence identifies an exact future admission,
  not an anonymous sleep. The unsealed `deadlines` field (key 41) keeps
  creation order, including renewals, and the exact signed logical instant.
- **`session_id` and `session_write_id`** use the same injective framed
  derivation as every other identity in this registry: one domain byte, then
  for each listed component `u32_le(length) ‖ bytes`. Owner path segments are
  separate fields, never a dotted display string.
  - inbound opening: `derive(0x06, 0x01 ‖ connector_path_segment* ‖ opening_root_id)`;
  - outbound opening: `derive(0x06, 0x02 ‖ connector_path_segment* ‖ emitting_id ‖ request_digest)`;
  - message write: `derive(0x07, session_id ‖ emitting_id ‖ message_digest)`.
  `SessionId` and `SessionWriteId` are opaque 16-byte logical references.
  Socket, address, negotiated protocol, Host, Worker, placement and arrival
  order never enter these recipes. Their lowercase hexadecimal text spelling
  is the sole `meta.ext.session_id` projection; causal record fields always
  carry the raw 16 bytes.
- **`closure` (EGRESS key 32) — the EXACT byteshapes.**
  The rule is « one call = exactly one closure ». The SEALED
  causal `status` (key 31) is exactly `1 open · 2 closed · 3 abandoned`. Key 32 is
  present **iff `status ∈ {closed, abandoned}`**, and its byteshape is
  fixed :
  - `status = 1 open` → key 32 **absent**.
  - `status = 2 closed`, `closure = response` → `[1, outputs]` (a
    2-element array). `outputs` is an ordered array of
    `[port_text, data_form]` drafts. It contains exactly one `exit`, last;
    no draft has an envelope id yet because the Execution constructs every envelope in
    the following atomic connector TRANSACTION.
  - `status = 2 closed`, `closure = timeout` → `[2]` (a **1-element**
    array — no second element, never `[2, null]` ; `l:timeout` emits
    `TimeoutExpired` as its own TRANSACTION).
  - `status = 3 abandoned` → `[]` (the **empty** array — present but
    zero-length ; recovery triage or bounded Closing resolved
    `OutcomeUnknown`, whose envelope is an `l:recover` TRANSACTION emission).
    These are the two supervision paths that may produce that declared case.
  - `discarded` (a LATE response arriving AFTER closure —
    delivery-semantics §4 / execution-model §6.3, **graph-invisible**) is
    **NOT a sealed EGRESS status** : it is a separate
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

**Injective causal projection.** Every causal distinction has a field:
record kind, four `meta.sys` fields, payload, writable metadata, exact
admission, consumed positions, ordered emissions, state delta, routes,
lifecycle, manager facts and exchange transitions. Scheduler due instants
are intentionally excluded, not derivable substitutes; explicit
`logical_time` observations are included. Minimally differing record pairs
test these distinctions.

**Sealed facts and unsealed circumstances.** The whole journal is immutable.
The split answers which bytes a causal comparison should require to agree.

| Sealed causal fields | Unsealed recovery or diagnostic fields |
|---|---|
| ingress and its payload; transaction inputs, state and emissions; Call/Session transitions; Loop/Retry facts; instance identity including time box; an observed `now()`; original quantities in annotated envelopes | exact deadline creation/due instants; original Call closure instant; diagnostic wall-clock durations and host information |
| compared by a golden | required recovery instants are retained even though a golden does not compare them |

Source annotations are semantic type metadata, not this unsealed ring.
`@measure` reads the quantity from the original sealed envelope using the
validated schema. Optional UI aggregates or future telemetry exports are
derived views, not new journal authorities.

**Deployment attestation.** Exact `env()`/`secret()` expressions and
selected evaluation sites hash; physical environment values and secret material
do not. A prepared HttpClient request seals a logical `SecretName`, never
the bearer itself. Recovery checks immutable environment/provider versions and
privacy-profile references/digests. A secret stays outside the journal in
clear. Such run-header evidence is control metadata, not a fourth causal
record kind.

M1 retains individual measurement facts without pricing or report aggregation.
An optional deployment cost profile remains an attested binding; no zero cost,
currency conversion or usage for a missing sample is inferred.
