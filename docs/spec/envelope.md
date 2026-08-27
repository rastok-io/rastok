# The Envelope — canonical form

> **Status:** normative — under review.
> This document is the **single source
> of truth** for the envelope's structure, identity, `meta.sys` reserved
> fields, the version stamp, and the storage / wire form. Other docs cite it
> rather than restating the field list.

An **envelope** is the unit that flows along edges and through transactions. It is
**immutable** : a transaction never mutates an envelope, it *emits* new ones (Mealy
frame, `f(state, input) → outputs`). The postal metaphor : `meta` is written
on the outside, `data` is sealed inside.

```
envelope = meta + data
  meta.sys   runtime-owned, READ-ONLY                       (this doc)
  meta.app   application-owned, read-write, INFERRED        (D-2026-08-10-1)
  meta.ext   external-boundary context, read-write          (D-2026-08-10-1/-4)
  data       the payload — any type, any encoding, lazy     (§5)
```

Exactly three zones, no other root: there is NO bare field
(`meta.customer_id` is invalid — no alias resolves it), no custom
namespace at the root, and no connector name in a meta path.
`app` is auto-available and its fields are INFERRED — from the
DEPLOYED GRAPH's statically visible RKG writes ONLY: types unify
across writers, a conflict refusing with both sites named; a read of
a name the graph writes nowhere refuses. There is no declaration
form. Three boundaries close the inference (D-2026-08-10-1):
a LAB never extends the production schema — fixtures, mocks, and
expectations are checked against the graph's resolved schema and
cannot add a field to it (nor touch the plan or the semantic hash);
world-owned values enter through `meta.ext` and the graph copies
them into `app` explicitly when it wants to keep them; and a GUEST
block (`jq`/Wasm) reads `meta` but never writes it in v1 — metadata
writes belong to the DSL, where the checker sees every key, so no
dynamic key can silently widen the schema. Within each zone the
flat keys are PREFIX-FREE: `app.audit` and `app.audit.origin`
cannot coexist (one is a leaf, the other makes it a branch of the
nested projection) — refused at check and re-verified by the
package validator. `ext`'s vocabulary belongs to
connector contract sheets (their `Populates` row, stdlib.md) plus
the standard protocol leaves (§5.2–5.4); handlers may rewrite declared
ext fields, never invent one. Each zone's leaves use the closed metadata
carrier `scalar | array<scalar> | map<text,array<scalar>>`. The map form is
one leaf, not a nested namespace; it exists for lossless repeated protocol
fields such as `meta.ext.http_headers`. Other nested/heavy data goes in `data`;
physically a field is ONE dotted string key inside its zone
(`"app.audit.origin"`), and the expression/guest projections nest by
splitting on dots. Propagation is uniform on a single-input
emission (`app` and `ext` copied, `sys` reminted, explicit writes
last-write-wins) and merges BY VALUE at joins — CommonOrAbsent per
field, `content_type` excluded (D-2026-08-10-2/-4). `meta.sys` is
the subject of this document.


## 1. `meta.sys` — the reserved namespace

`meta.sys` is **fully read-only and runtime-owned**. The DSL never assigns to
it (see §6 for the one apparent exception — emission constructors — and why it
is not a write). It has two tiers.

### 1.1 Tier 1 — the spine (always present)

Five fields, present on every envelope, owned by the spec's schema (so any
Rastok tool of any version can read any envelope's header without knowing the
emitting graph) :

| Field | Type | Meaning |
|---|---|---|
| `meta.sys.id` | `uuid128` | unique identity, **derived** (§2) — never random |
| `meta.sys.parent_id` | `uuid128?` | the immediate ancestor envelope ; `null` at inbound. The chain of `parent_id` **is** the lineage (a linked list). |
| `meta.sys.origin_id` | `uuid128` | the lineage root — the inbound ancestor's `id`. A root envelope is its own origin (`origin_id == id`). Exported as the OpenTelemetry trace id. Never `null`. |
| `meta.sys.role` | `enum (extensible)` | the envelope's role : `default`, `compensate`, … Default value `default`. Omissible on the wire when `default`. |
| `meta.sys.graph_version` | `u64` | sequential build number of the emitting graph ; the **manifest** (§4) maps it to the content hash, dev semver, commit, and schemas. |

The external content type is NOT a `sys` field: it is the standard
`ext` field `meta.ext.content_type` (§5.2, D-2026-08-10-4) — the
full Content-Type string, parameters included, absent when the
payload is a structured in-memory value.

`uuid128` is a fixed 128-bit value. (The values are blake3-derived, not RFC
UUIDs — see §2 — but the physical type is a 128-bit identifier.)

### 1.2 Tier 2 — birth stamps (aspect scopes)

Minted on envelopes **born** inside the corresponding aspect scope ; envelopes
born outside carry nothing (zero overhead) :

| Fields | Present when | Spec |
|---|---|---|
| `meta.sys.saga_*` (`saga_id`, `saga_stack`) | **birth stamps** : minted on envelopes *born* inside a `: Saga` scope | [effects-and-sagas.md](effects-and-sagas.md) |
| `meta.sys.loop_*` (`loop_id`, `loop_stack`, `iter_count`) | **birth stamps** : minted on envelopes *born* inside a `: Loop` scope | [loop.md](loop.md) |

**The Tier-2 frame.** Tier-2 fields
are **birth-context stamps**, immutable like everything else on the envelope :
set at mint time when the emitting transaction runs inside the scope, absent
on envelopes born outside, and they simply travel wherever the wires go
(nothing strips them — nothing *can* : edges are wires, envelopes are
immutable ; and it is honest provenance : that envelope *is* the scope's
product). **In the language**, the aspect context is *instance* context —
inside the lexical scope, the Rastok expression environment exposes `meta.sys.saga_id`,
`saga_stack`, `loop_id`, `loop_stack`, `iter_count` **resolved from the
aspect instance** (exactly like the contextual `instance.key`) : always
present inside — even while processing the scope-*creating* envelope, which
was born outside — and not in the env at all outside the scope (compile
error, even with `.?` — no abstraction leak). Lifecycle facts
(`saga_state`/`loop_state` transitions, `*_started_at`) are **journal- and
observability-side only** — wall-clock never enters the data plane.

That is the **entire** `meta.sys` surface. Any other `meta.sys.*` field
is outside the current contract — see §9.


## 2. Identity — derived, zero per-envelope randomness

Every id is **derived** : no random bits enter any id, ever. The full
recipe includes the emitting node and port so fan-out cannot collide:

```
id        = blake3( node_path ‖ trigger ‖ parent_id ‖ emission_port ‖ emission_index )
inbound   : id = blake3( connector_path ‖ inbound_seq )    (the root ; parent_id = null)
origin_id = the id of the lineage's inbound root
```

**The `‖` framing — BLAKE3 is part of the contract:**

```
input      = domain_tag ++ frame(field)*
frame(f)   = u32_le( len(f) ) ++ bytes(f)          // length-prefixed : INJECTIVE
domain_tag = 0x01 (root, 2 fields)          |  0x02 (emission, 5 fields)
             0x03 (root + boot-epoch, 3 fields : connector_path, inbound_seq,
                   epoch u64 LE — the `durability = none` mode)
             0x04 (root + replica — RESERVED byte ; multi-host semantics are deferred)
             0x10 (call, 2 fields : delivery_ref, request_digest)
             0x20 (delivery reference, 2 fields : canonical_edge_text, envelope_id)
             0x21 (instance lifetime, 2 fields : canonical instance-address bytes,
                                                    generation u64 little-endian)
             0x22 (instance Closing phase, 3 fields : canonical instance-address
                                                    bytes, generation u64 LE,
                                                    timer kind byte)
id         = first 128 bits of blake3(input)
encodings  : paths/triggers/ports = UTF-8 ; parent_id = its 16 raw bytes ;
             inbound_seq = u64 LE ; emission_index = u32 LE
```

Length-prefixing kills concatenation ambiguity (`"ab"‖"c"` ≠ `"a"‖"bc"`) ;
the domain byte separates root from emission derivations (different
arities could otherwise realign).

The five components :

- **`node_path`** — *who emits*. The emitting node's fully-qualified
  composition path (`"route"`, `"sessions.route"`, `"sessions.inner.retry"`),
  **relative to the composition root — the `: App` graph, whose own name is
  EXCLUDED** so renaming the deployment entry does not perturb every
  descendant id,
  **static, without the logical instance address** (one envelope enters one
  address of a given node for a delivery; the address therefore adds no
  envelope uniqueness and remains per-hop journal data). Without this component, two
  nodes fed by the same fan-out envelope would derive colliding child ids.
- **`trigger`** — *what fired*. Namespaced to make port names and lifecycle
  names collision-free forever :
  - `p:<port>` — the arrival port that triggered the transaction. For a join
    `on (a, b)`, the port of the **triggering** envelope (the one that
    completed presence under mailbox-FIFO order).
  - `l:<lifecycle>` — `l:start`, `l:closing`, `l:stop`, `l:reply`,
    `l:exit`, `l:compensate`, `l:next`, `l:timeout`, `l:evict`, `l:recover` for transactions not triggered by a
    port arrival
    (§3.1 ; the last two are defined by
    [execution-model.md](execution-model.md) §6-§7).
  Without this component, an `on start` transaction and the `on in` transaction of the
  instance-creating envelope would collide ; so would two handlers of one node
  fed by the same fan-out envelope on two ports.
- **`parent_id`** — *the cause* (§3.1 : there is a causal envelope for every
  transaction kind).
- **`emission_port` / `emission_index`** — *which output*. Within a transaction,
  outputs are emitted on declared ports in a deterministic order (the
  as-if-run-to-completion contract, [execution model](execution-model.md)) ;
  the index counts emissions per port in handler-text order. The precise
  index rule is pinned by the execution-model spec.

**The delivery reference.** One envelope delivered along one edge is
one *delivery*, and its identity is the pair `(canonical edge text,
envelope id)`, condensed as a derived id :

```
delivery_ref = blake3-derive( 0x20, canonical_edge_text ‖ envelope_id )
```

The delivery reference — never the envelope id alone — is the
identity of every per-delivery obligation : the call budget a
delivery arms (identity.md §3, `timers`), the queue and pending
lookups that resolve an expiry, and the derivation of the call
identity below. A fan-out of one envelope to two edges is two
deliveries, two references, two obligations — two ports of one
connector included.

**The instance-lifetime reference.** One lifetime obligation belongs to one
complete instance reference, not merely to its reusable logical address:

```
instance_lifetime_ref = blake3-derive(
  0x21,
  canonical_instance_address_bytes ‖ u64_le(generation)
)
```

The full instance reference is also carried by the dated event. The derived
reference is the stable timer identity sealed in transaction key 37; the due
time remains replay-derived. Re-arming idle within one generation therefore
keeps one logical obligation while its causal envelope changes, and an event
from a closed generation cannot target its successor.

**The call identity.** An outbound call is caused by exactly one
delivery carrying exactly one prepared request, and its internal
identity derives from both :

```
call_ref = blake3-derive( 0x10, delivery_ref ‖ request_digest )
```

`request_digest` is the digest of the EXACT protocol bytes prepared
for the partner (delivery-semantics §4.2) ; its definition never
absorbs routing information — the delivery reference carries that.
Two ports of one connector receiving one envelope with identical
bytes therefore derive two distinct calls.

Wiring rule : the **same source port wired twice to the same input port is a
compile error** (a duplicate edge is always a typo — and it would defeat the
recipe).

- **`inbound_seq`** : a monotone counter per inbound Connector instance. Its
  source by mode :

| Mode | `inbound_seq` source | Determinism |
|---|---|---|
| Simulation | the fixture (event N of the test set) | total — identical ids every run ⇒ byte-stable golden traces |
| Prod, `durability ≥ local` | a **durable** counter, committed with the inbound accept | total — replay re-derives identical ids from the journal |
| Prod, `durability = none` | RAM counter + boot epoch in the derivation | unique, but not stable across restarts — and that is fine : no journal, no replay |

**Reserved multi-host property.** When inbound scales
to replicas, each replica's identity joins the root derivation —
`blake3( connector_path ‖ replica ‖ inbound_seq )` — so `(connector, replica,
seq)` triples never collide. In single-process execution, `replica` is
empty.

### 2.0 The honest determinism contract — journal-relative

The derivation above is deterministic *given its inputs*. But one input —
`parent_id` at a **multi-input join** — is the outcome of a **race** : two
branches of the graph deliver to the same mailbox, and which envelope arrives
last (the trigger, hence the parent of the join's output) depends on
scheduling and external timing, not on the graph. Connector responses and
`within` windows race the same way. This is the classic timed-merge
(Brock-Ackerman) situation : **a non-linear graph is *not* a pure function of
its inbound history.**

The contract Rastok actually makes — the same one the durable-execution
industry makes — is **replay-determinism relative to the journal** :

```
LIVE    races happen ──► the per-hop journal records their outcomes
                         (arrival orders, supervision decisions, inbound events)
REPLAY  the journal is re-read ──► no races re-run ──► identical run, identical ids
SIM     the fixture IS the journal ──► fully deterministic, byte-stable golden traces
```

So : **zero randomness in the ids ; all nondeterminism captured in the
journal ; the run is a pure function of `(graph, journal)`.** Under
`durability = none` the journal lives in memory or as a trace export:
simulation replay works fully, but replaying a *production incident*
requires a durable journal. Said plainly: in-memory durability gives
deterministic simulation and in-process traces, not post-mortem
production replay.

**Hash algorithm: BLAKE3, fixed by this contract.** Changing it would
change every derived id and break replay. It was chosen for speed on small
inputs (one hash per emission, on the hot path), a native XOF that truncates
cleanly to 128 bits, and a maintained Rust reference implementation. If it ever
had to change, it would require a new header format and an explicit
compatibility contract.

### 2.1 What derived ids buy

1. **Structural dedup.** A replayed transaction re-emits the *same* id from the same
   inputs ⇒ downstream Connectors deduplicate by id. This is the foundation of
   effectively-once delivery ([delivery-semantics.md](delivery-semantics.md)).
2. **`meta.sys.id` is a natural idempotency key** for `irreversible`
   Connectors — as an *integrity* handle. It is NOT the idempotency key :
   the key names the **business operation** and stays **stable across
   retries** ([delivery-semantics.md §4.3](delivery-semantics.md) — a
   per-call-varying key is the double-pay trap ; the per-attempt form is
   the documented escape, [loop.md](loop.md) §7.3).
3. **Byte-stable golden traces.** Two simulation runs produce *identical*
   envelopes ⇒ reference traces diff at the byte level. "Simulation = the spec"
   becomes literal.
4. **Stable debugging identity.** "Envelope X" has the *same* id in every
   re-run, unlike a random UUID.


## 3. Identity semantics — where a new id appears

- **Edge transport = same envelope, same id.** An edge is a wire, not a
  computation. The envelope leaving `a.out` and arriving at `b.in` is the same.
- **Every transaction emission = a NEW envelope with a new derived id** — even a
  content-identical passthrough (`out = .`). A transaction's output is a
  new fact (a span, a commit, a journal entry, an identity), even if its
  content equals the input. New id ≠ new allocation : `data` bytes are shared
  by reference (immutability makes sharing safe) ; the cost is one blake3 of
  ~40 bytes.

On a chain `a -> b -> c` :

```
inbound ─► E0 ──a──► E1 ──b──► E2 ──c──► E3
          id0      id1        id2        id3        each id = blake3(node ‖ trigger ‖ parent ‖ port ‖ index)
          parent: ∅  id0       id1        id2        the parent chain IS the path through the graph
          origin_id : id0  (constant on the whole lineage)
```

Composed cases :

- **Broadcast** `(x, y) = in` → two envelopes, different ids (different
  `emission_port`), same `parent_id`, same `node_path`.
- **Edge fan-out** `a -> { x ; y }` → **no new envelope** (edges are wires) :
  the same envelope, same id, arrives at both `x` and `y`. Their subsequent
  emissions differ by `node_path`. Adding a subscriber never changes the ids
  existing subscribers see.
- **Join** `on (a, b)` → the emitted envelope's `parent_id` is the
  **triggering** envelope (the one whose arrival completed presence) and its
  `trigger` is that envelope's arrival port ; the join partners contributed
  through the owning Operator instance's join buffer, recorded in the per-hop
  journal (§7).
  **The output inherits the trigger's `origin_id`** — when two lineage roots
  feed one join, the other root stays reachable through the journal (the
  3-piece provenance) ; there is no "merged" origin, and the exported OTel
  trace follows the trigger side. *(Made explicit by reference simulation 2.)*
  **Corollary:** request/response correlation must never ride on
  `origin_id` across a join — `HttpServer { mode = respond }` correlates by
  lineage only on join-free return paths, and requires an explicit
  `correlate` field otherwise ([stdlib.md §3.1](stdlib.md)).
- **Loop** (`: Loop` re-injection) → each call is a new envelope emitted by
  the Loop-carrying node, trigger `l:next`, `parent_id` = the previous call's
  output. The parent chain *is* the iteration history.

### 3.1 Transactions without a port arrival — trigger and parent

Every transaction has a **causal envelope** ; the `l:` trigger namespace covers the
transactions that no port arrival triggers :

| Transaction | `trigger` | `parent_id` (the cause) | Emitter (`owner_path` in the record) |
|---|---|---|---|
| `on start` | `l:start` | the envelope whose admission created the Node instance | the node itself |
| Instance close request | `l:closing` | the envelope whose handler requested `stop`, or the envelope that armed the winning lifetime deadline | the closing owner itself |
| `on stop` via the `stop` action | `l:stop` | the envelope being processed when `stop` ran | the node itself |
| `on stop` via lifetime expiry | `l:stop` | **the envelope that armed the deadline** — the last admitted envelope (`idle`), creator (`fixed`), or creating admission of the processing-time box (`tumbling`). Always exists; there is no phantom tick | the node itself |
| Inbound Call terminal | `l:reply` | the committed root request that opened the Call | the inbound Connector instance that admitted it |
| Session terminal | `l:exit` | the envelope that opened the logical Session | the Connector instance that owns the Session |
| Loop re-injection | `l:next` | the previous call's output — the chain *is* the iteration history | **the `: Loop`-carrying node** |
| Saga compensation envelope | `l:compensate` | **the envelope explicitly routed to `Saga.rollback`** (or the saga's `SagaExpired` timeout event); for compensations 2..n: the previous compensation's ack (the chain IS the reverse rollback order) | **the `: Saga`-carrying node**, port `compensate` (reserved), index 0 |
| Budget expiry (call `timeout`, `Saga.timeout`) | `l:timeout` | **the envelope that ARMED the budget** — the request (call) ; the instance-creating envelope (saga) | the Connector node / the `: Saga` node |
| Join-buffer eviction | `l:evict` | the evicted envelope ([execution-model.md §7](execution-model.md)) | the Operator node |
| Recovery resolution of an unknown outbound | `l:recover` | the request envelope whose outcome is unknown ([delivery-semantics.md §4.4](delivery-semantics.md)) | the Connector node |

The last two rows state the principle for **synthesized envelopes** : a
runtime layer (loop manager, saga manager, supervision plane) never emits in its own name —
the emission is attributed to the **aspect-carrying node**, with a lifecycle
trigger from the aspect's vocabulary. The recipe has no out-of-graph emitters.
The saga row's scheme is fixed: compensations are
**sequential transactions** of the `: Saga` node — port `compensate`
(reserved), index 0, parent = the rollback's cause for the first, then the
previous compensation's ack — the parent chain *is* the reverse rollback
order, and the rollback is
journal-driven by definition, so replay re-reads the same order and re-derives
the same ids.


## 4. The version stamp — one field, the manifest does the rest

`meta.sys.graph_version` is a **`u64` sequential build/deploy number** per app.
Everything else about versioning is derived through the **manifest**, an
infrastructure artifact the runtime already needs (it holds the schemas
required to decode the Avro zones — §5).

```
manifest[graph_version] = {
  content_hash : blake3 of the canonical .rkg   // integrity — re-hashable, cannot lie
  semver       : dev-declared `graph <name> : App { version = "1.4.0" }`   // the human version
  commit       : VCS ref, recorded by CI        // bridges hash ↔ source, VCS-agnostic
  schemas      : the data + per-namespace schemas of this build
}
```

Why a number on the envelope and not a hash : it is small (8 bytes), **ordered**
(a hash cannot answer "did build #42 precede #43 ?"), and human-facing ("deploy
#42"). Why not the semver on the envelope : a semver can lie (a dev forgets to
bump ; two graphs both claim "1.4.0") — for replay integrity you need the
content-derived identity, which lives in the manifest and is re-hashable.

So **humans say "v1.4.0", the runtime uses the number, the manifest bridges
them**. `spec_version` and per-namespace schema fingerprints are *derived* via
the manifest, not carried per-envelope. During a rolling upgrade, envelopes
from different builds coexist, each self-labelled by its number. The retained
manifest schema makes an old envelope decodable; it does not silently make its
logical type acceptable to a new graph. A schema change is crossed by an
explicit typed Operator which consumes the old value and emits a new envelope
under the current `graph_version`, with ordinary parent lineage. Journal
migration onto a different semantic hash remains deferred and refuses typed
until that execution contract lands ([types.md §6](types.md)).

In single-process mode the manifest is a local file with one deployer.
Scaled multi-host allocation of the number requires a separate cluster
contract and is deferred.


## 5. Storage / wire form — "persisted form = transport form"

There is **one** representation : what travels the wire is exactly what is
persisted to the journal, durable state, and audit. No second form, no
coherence debt.

```
envelope (storage = wire) =
  header      meta.sys              → Avro ; schema owned by the SPEC
  zones       meta.app + meta.ext   → Avro ; app schema owned by the APP (graph_version manifest),
                                      ext schema by the connector sheets + the standard fields
  data        (encoding_tag, bytes) → ANY registered codec — lazy
```

- The **header has a spec-owned schema** : any Rastok tool, any version, reads
  any envelope's `meta.sys`. This is what makes cross-version audit and
  debugging possible.
- **`data` is tagged bytes.** The static type (what `data` *means* — `bytes`,
  `struct`, `table<…>`) comes from the graph schema via `graph_version`. The
  `encoding_tag` (how it is *represented now*) is dynamic, per-envelope, drawn
  from the runtime's internal representations : `raw | avro | arrow-ipc`. `meta.ext.content_type` is never an internal decode tag: the core
  decodes with `encoding_tag` ALONE. The v1 regime (D-2026-08-10-4):
  a structured external body (HTTP JSON) is decoded EAGERLY at the
  frontier by the connector — `Materialized(variant)` — while `raw`
  carries opaque bytes whose only structural readings are UTF-8 text
  or the bytes themselves; a possible future `Encoding::Json` would
  make JSON lazy WITHOUT consulting Meta. A JSON body is never
  parked under `raw` where a structural access would materialize it
  as raw text.

### 5.0 Four planes, never confused

| Plane | Answers | Carried by |
|---|---|---|
| Logical type | what do ports and Rastok expressions see? | the graph schema (`text`, `Invoice`, `variant`, …; [types.md](types.md)) |
| Internal encoding | how are the bytes represented NOW? | `Data::Encoded.encoding` (`raw \| avro \| arrow-ipc`) |
| Connector codec | how does a boundary serialize/deserialize? | the connector's `codec` attribute |
| External content type | what does the outside world call this exchange? | `meta.ext.content_type` (§5.2) |

The content type is neither a logical type nor an internal decode
tag: a `json! { … }` literal is a materialized `variant` VALUE and
carries no content type — only a boundary that serializes it mints
bytes and a matching `content_type`.

### 5.1 Lazy rules (Memory-Representation Polymorphism)

1. **No structural access → no transcode, ever.** An opaque `bytes` payload
   routed on `meta` alone is never parsed nor re-encoded — the bytes traverse
   untouched; an OGG payload stays OGG forever. (The pass-through ideal for
   structured bodies awaits a lazy `Encoding::Json`; in the v1 regime a JSON
   body is decoded eagerly at the frontier — D-2026-08-10-4 — because parking
   it under `raw` would make a structural access materialize raw text.)
2. **Access → transcode once** to the guest-native form (`jq!` on a document →
   parse ; `sql! {}` on a table → Arrow, once).
3. **A value built by the graph** (output of a `jq!`, a join) is born in memory
   with no original bytes ; on persistence it takes its guest-native form :
   `avro` for documents, `arrow-ipc` for tables, raw bytes for `bytes`/`text`.
4. **Persistence stores the current form** — never a forced transcode at write.
   The journal is therefore a self-describing mix (some json, some arrow, some
   opaque), each entry tagged. Replay re-delivers identical bytes.

There is no single "canonical encoding" for `data` — an OGG stream has no Avro
form to promote to ; the bytes *are* the value. The canonical thing is the
**framing** (the three zones + the per-envelope `encoding_tag`), not a forced
codec for `data`.


### 5.2 `meta.ext.content_type` — the payload-coupling invariant

`content_type: media_type?` is the standard reserved `ext` field: the FULL
external Content-Type (parameters included), written by inbound
connectors, by the whole-payload constructors (`asset`, `base64` —
D-2026-08-10-5), or by a handler. It is coupled to the payload, not
inherited like ordinary context: emission construction (one atomic
order, D-2026-08-10-4) fuses/copies `app` and `ext` WITHOUT it,
decides the payload, then derives it — a kept witness `Payload(x)`
takes input x's value; FRESH data leaves it ABSENT; an explicit
write in the same emission wins. The decision follows the VALUE
(the keyed witness), never the spelling: `out = .`, `out = data`,
`out = a.data`, a local carrying the witness, a ternary's taken arm
all preserve representation and content type together. Outbound, a
serializing codec dictates and writes the produced value; a byte
pass-through uses the current value if compatible, else the
connector's configuration, else `application/octet-stream`; a
codec/content-type contradiction is a typed boundary outcome, never a silently
false header.

The value is one canonical concrete media type, not arbitrary text and not an
HTTP media range. A boundary parses it through the same `media_type` authority
used by the expression constructor. A malformed peer `Content-Type` remains
available byte-for-byte in `meta.ext.http_headers`, but does not fabricate the
typed `content_type` leaf.

### 5.3 `meta.ext.session_id` — logical boundary-session correlation

D-2026-08-25-1 reserves `session_id: text?` as the M1.6 session-contract
projection. A WebSocket Client/Server Driver writes the same value on every
peer message and on the sole terminal session `exit`. It is derived from the
journal's opaque logical `SessionRef`; it is never a file descriptor, socket
address, negotiated HTTP version, Host or Worker id.

The field is in `meta.ext`, not the closed `meta.sys` spine, because it is
boundary-protocol context that a graph may deliberately preserve or replace
when routing a stored/root-mixed response. Direct replies retain causal
session routing independently of the writable projection. An explicit
`correlate = meta.ext.session_id` asks the Connector to verify and use that
logical token; it never authorizes a physical address lookup from graph data.
The projection is the lowercase 32-hex-character spelling of the reference's
16 bytes. H1 makes this executable in categorical package v12. The journal
derives inbound and outbound openings under identity domain `0x11`, and each
graph-to-world message write under `0x12`; the exact framed recipes and causal
record forms are fixed in [identity.md §3](identity.md).

### 5.4 Standard HTTP and WebSocket projections

D-2026-08-25-2 reserves the following optional leaves alongside
`content_type` and `session_id`:

| Leaf | Logical type | Meaning |
|---|---|---|
| `content_type` | `media_type` | canonical concrete Content-Type including parameters; absent when a peer spelling is invalid |
| `http_headers` | `map<text,array<bytes>>` | lossless fields; non-empty lowercase ASCII token names and value arrays, repeated values in wire order, no HTTP/2-3 pseudo-fields |
| `http_method` | `text` | admitted request method |
| `http_uri` | `text` | admitted or selected request URI under the contract |
| `http_version` | `text` | observed `http/1.1`, `http/2`, or `http/3`; never semantic identity |
| `http_status` | `int` | observed response status; the Driver validates the protocol's range |
| `websocket_origin` | `text` | admitted Origin value when present |
| `websocket_subprotocol` | `text` | the one negotiated subprotocol when present |

The array in `http_headers` is required even for one value because HTTP field
repetition and order can be semantically relevant (`set-cookie` is the
canonical example). Header values are bytes rather than text; a Driver never
destroys a valid non-UTF-8 field value to populate metadata. The Driver
normalizes field names to lowercase after protocol validation. Pseudo-fields,
connection handles, HPACK/QPACK indexes and library objects are physical
protocol state and never become envelope data.

These leaves are observations or response-routing material. A peer-provided
spelling cannot override a runtime-minted `session_id`, and a graph write does
not retroactively change which HTTP version or socket carried a committed
exchange. The complete map participates in ordinary `meta.ext` typing,
identity, merge and explicit-attribution rules once present.

## 6. Writing `meta.sys` — there is no write

`meta.sys` is **read-only, absolutely — including in simulation**. The
DSL cannot assign `meta.sys.*`. Three things
that look like writes are not :

1. **Inbound.** A boundary Connector sets the header from the transport (it is
   the runtime acting, not the app).
2. **Emission constructor.** Content metadata is ordinary `ext`
   writing now — `out = { data = …, meta.ext.content_type = "image/png" }`,
   or the whole-payload constructors (`asset("…", "image/png")`,
   `base64!("image/png") { … }`). Nothing reaches `meta.sys`: the
   payload-coupling invariant (§5.2) derives or strikes
   `content_type` when `data` is replaced — a reformulation of the
   old "runtime nulls the hint" rule, executed at the same atomic
   construction point.
3. **Simulation fixtures.** A fixture seeds inbound state through `meta.ext`
   (writable) or a runtime-interpreted braced envelope-form builder — **never** by
   assigning `meta.sys.*`. Simulation stands in for the runtime, but it does so
   through the same constructor surface, not by violating the invariant. This
   keeps one mental model and lets the type checker enforce read-only with no
   exception.


## 7. Provenance — three pieces

The envelope carries the **minimal spine** ; the **journal** carries the full
graph of events. The journal itself is specified by
[delivery-semantics.md](delivery-semantics.md) §6 — **three CAUSAL record
types** (plus the derived, non-causal SNAPSHOT — rebuildable, excluded from
traces and replay) :
INBOUND (the world enters, `inbound_seq` committed atomically with the
accept), TRANSACTION (the atomic unit : consumed positions, state delta,
emissions, `committed_at`), OUTBOUND (the graph acts on the world : `open` →
`closed` XOR timeout | `discarded`). Supervision decisions ARE transaction
records (`l:*` triggers) ; clock ticks ARE inbound records. Three pieces
together reconstruct everything :

| Piece | Where | Gives |
|---|---|---|
| `parent_id` linked list | envelope | the primary lineage, walkable via the journal in `O(depth)` |
| join contributions | journal, per-hop — the **`join_inputs`** field of the joining TRANSACTION record's causal core ([delivery-semantics.md §1](delivery-semantics.md)) | the full DAG (which envelopes fed which transaction) |
| logical instance reference at each hop | journal, per-hop | which Node address and generation received the delivery |
| `committed_at` (UTC) | journal, per-hop — an **observation annotation** on the record (outside the causal core / the trace fingerprint, [delivery-semantics.md §1](delivery-semantics.md)) | the **regulatory timestamp** — commit-authoritative, single source of truth |

The `parent_id` pointer *references* history, it does not *transport* it :
walking the chain requires the journal (`durability ≥ local` ; `global` for cross-host reach). Without a
journal (`durability = none`), `parent_id` is still useful locally (immediate
correlation, debug) but the chain only walks back as far as upstream envelopes
still live in memory.

**The journal's structure (fields, access rules) is specified separately** —
it is the home of `seq`, `committed_at`, the logical instance reference per hop, and the
join contributions. See [delivery-semantics.md](delivery-semantics.md) (#4) and
the audit dossier (#8). This doc only states *what moved there* and *why*.


## 8. The determinism boundary (why these fields, and not others)

Three principles drove every inclusion and every removal :

1. **The envelope carries identity ; the journal carries events.** Anything
   that is a *fact of a transaction* (when it committed, in what arrival position, at
   what logical instance reference, who else fed the join) is per-hop and lives in the
   journal — not on the envelope. Anything that is the envelope's own *identity*
   (its derived id, its lineage pointers, its role, its content type, its build
   stamp) is on the envelope.
2. **No hidden clock in the data plane.** A handler must not be able to read
   wall-clock and branch on it without going through a `Clock`. A
   readable `created_at` on the envelope was exactly such a leak — so it was
   removed (its regulatory purpose moved to the journal's `committed_at`).
   Wall-clock observation belongs to the non-deterministic
   observation/supervision plane, which injects its decisions back as journaled
   events — the same mechanism as `Clock` ticks. (That plane is
   specified with the execution model, #2.)
3. **Determinism is a property of `(graph, journal)`, not of the graph
   alone** (§2.0). Individual transactions are deterministic ; multi-input joins,
   Connector responses and `within` windows are **timed merges** — races whose
   outcomes only the journal captures. The journal records them ; replay
   re-reads them ; the simulation fixture plays the journal's role. Nothing
   nondeterministic is ever *embedded in the envelope* — it is journaled
   beside it.


## 9. Fields removed or renamed (migration from older docs)

Older docs and examples reference `meta.sys` fields that no longer exist. The
canonical mapping :

| Old field | Status | Where the concern went |
|---|---|---|
| `envelope_id` | renamed | → `id` (now derived, §2) |
| `seq` | removed from envelope | → per-hop journal ; ordering = mailbox FIFO per instance reference (§7, and execution model #2) |
| `created_at` | **removed** | hidden clock leak (§8) ; regulatory timestamp → journal `committed_at` |
| `correlation_id` | removed | → Connector-internal protocol ids + logical instance address + a `meta.app` echo-token (re-addable later, additive) |
| `trace_id` | merged | → `origin_id` *is* the OTel trace id (§1.1). There is **no** `meta.sys.trace_*` field, now or planned. |
| `direction` | removed | no use case (the stream-protocol "direction" of a *Connector* is a different thing and stays) |
| `action` (`forward`/`compensate`) | renamed | → `role` (extensible enum, §1.1) |
| `type` / `encoding` | removed as flat fields | typing is per-zone (data, meta.app, meta.ext), defined by the graph schema via `graph_version` ; the logical/physical split survives as a *schema* concept, not envelope fields |
| `kind` | renamed | → `role` |
| `mime` | renamed, relocated | → `meta.ext.content_type` (full Content-Type, parameters included — D-2026-08-10-4 ; the earlier `meta.sys.mime_type` and its NiFi-lineage name die) |


## 10. Related docs

- [effects-and-sagas.md](effects-and-sagas.md) — Tier-2 `saga_*` ; `role = compensate`.
- [loop.md](loop.md) — Tier-2 `loop_*` ; `id` as a per-call idempotency key.
- [execution-model.md](execution-model.md) — the mailbox/ordering contract,
  `emission_index`, the dated-events queue, `l:timeout` / `l:evict`.
- [delivery-semantics.md](delivery-semantics.md) — the journal, `committed_at`, dedup by `id`, effectively-once.
- [grammar.md](grammar.md) — the envelope constructor grammar.
