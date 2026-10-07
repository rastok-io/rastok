# The Envelope — canonical form

> **Status:** normative — under review.
> This document is the **single source
> of truth** for the envelope's structure, identity, `meta.sys` reserved
> fields, the version stamp, and the storage / wire form. Other docs cite it
> rather than restating the field list.
> Format 20 has one current representation, without compatibility
> readers. The [cutover inventory](../architecture/human-review-v20-identity.md)
> records the reviewed pre-public change and its single blessing.

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
is not a write). Its complete surface is the four fields below.

### 1.1 The four runtime-owned fields

Four fields, owned by the format's schema. A reader of this format can
read the header without knowing the emitting graph. The parent is absent
at a root:

| Field | Type | Meaning |
|---|---|---|
| `meta.sys.id` | `uuid128` | unique identity, **derived** (§2) — never random |
| `meta.sys.parent_id` | `uuid128?` | the immediate ancestor envelope ; `null` at inbound. The chain of `parent_id` **is** the lineage (a linked list). |
| `meta.sys.origin_id` | `uuid128` | the lineage root — the inbound ancestor's `id`. A root envelope is its own origin (`origin_id == id`). Available to a future telemetry projection as a trace correlation key. Never `null`. |
| `meta.sys.graph_version` | `u64` | sequential build number of the emitting graph ; the **manifest** (§4) maps it to the content hash, dev semver, commit, and schemas. |

The external content type is NOT a `sys` field: it is the standard
`ext` field `meta.ext.content_type` (§5.2, D-2026-08-10-4) — the
full Content-Type string, parameters included, absent when the
payload is a structured in-memory value.

`uuid128` is a fixed 128-bit value. (The values are blake3-derived, not RFC
UUIDs — see §2 — but the physical type is a 128-bit identifier.)

### 1.2 Extension context is not envelope metadata

Loop run identity, application ordinals and Retry operation state belong to
journal packets. `Loop.state` is a contextual binding, not a fifth `meta.sys`
field. Saga remains a future Extension and has no allocated envelope field.
There is no role, birth-stamp tier, loop counter or compatibility alias in
`meta.sys`. An unknown field refuses at check.

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
input      = u8::from(kind) ++ frame(field)*
frame(f)   = u32_le( len(f) ) ++ bytes(f)          // length-prefixed : INJECTIVE
id         = first 128 bits of blake3(input)
encodings  : paths/triggers/ports = UTF-8 ; parent_id = its 16 raw bytes ;
             inbound_seq = u64 LE ; emission_index = u32 LE
```

`envelope::IdKind` is the closed registry of the implemented recipes below.
Each `#[repr(u8)]` variant carries its explicit wire byte; `From<IdKind>`
projects that discriminant, never declaration order. `TryFrom<u8>` refuses
all other bytes as `UnknownIdKind`. This is the compact format-20
registry. Decoder names are specified in §5.1; runtime attestation is deferred.

| Kind | Byte | Length-prefixed fields, in order |
|---|---|---|
| `Root` | `0x01` | Connector path, ingress sequence |
| `Emission` | `0x02` | Node/owner path, trigger, parent id, port, per-port emission index |
| `LoopAdmission` | `0x03` | run id, iteration, consumed command id |
| `RetryAdmission` | `0x04` | operation id, next attempt ordinal, previous winning Call id |
| `Call` | `0x05` | delivery id, prepared-request digest |
| `Session` | `0x06` | subtype; path segments; opening root id (inbound), or emitting id and request digest (outbound) |
| `SessionWrite` | `0x07` | Session id, emitting id, message digest |
| `LoopRun` | `0x08` | instance identity bytes, generation, opening-admission kind, exact opening id, entry port (empty for a delivery) |
| `LoopCommand` | `0x09` | run id, iteration, command-kind/Done-provenance pair, cause id |
| `RetryOperation` | `0x0a` | instance identity bytes, generation, entry port, exact opening delivery id |
| `Delivery` | `0x0b` | canonical edge text, envelope id |
| `InstanceLifetime` | `0x0c` | instance identity bytes, generation |
| `InstanceClosing` | `0x0d` | instance identity bytes, generation, explicit deadline-kind byte |
| `InstanceStop` | `0x0e` | source owner kind, source port, source identity/generation, scope identity/generation, target identity/generation, cause id |
| `Decoder` | `0x0f` | canonical decoder name as UTF-8; attestation implementation deferred to M2 |

Generation, sequence and iteration/attempt ordinals use u64 little-endian;
emission indices use u32 little-endian. Tags/subtypes are bytes; ids are
their 16 raw bytes. Session subtype 1 denotes inbound opening, 2 outbound.
A root's current single-process recipe does not include a boot epoch or
replica. Cross-run/global uniqueness is not claimed. The Loop run recipe uses the
exact opening admission, not only its business origin ([loop.md](loop.md)).

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
    `l:exit`, `l:next`, `l:retry`, `l:timeout`, `l:evict`, `l:recover` for transactions not triggered by a
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
delivery_id = blake3-derive( 0x0b, canonical_edge_text ‖ envelope_id )
```

The delivery reference — never the envelope id alone — is the
identity of every per-delivery obligation : the call budget a
delivery creates (identity.md §3, `deadline_ids`), the queue and pending
lookups that resolve an expiry, and the derivation of the call
identity below. A fan-out of one envelope to two edges is two
deliveries, two references, two obligations — two ports of one
connector included.

**The instance-lifetime reference.** One lifetime obligation belongs to one
complete instance reference, not merely to its reusable logical address:

```
instance_lifetime_id = blake3-derive(
  0x0c,
  canonical_instance_address_bytes ‖ u64_le(generation)
)
```

The full instance reference is also carried by the dated event. The derived
reference is the stable deadline identity sealed in transaction key 24; the original due
instant is retained unsealed and restored exactly. Renewing idle deadlines within one generation therefore
keeps one logical obligation while its causal envelope changes, and an event
from a closed generation cannot target its successor.

**The call identity.** An outbound call is caused by exactly one
delivery carrying exactly one prepared request, and its internal
identity derives from both :

```
call_id = blake3-derive( 0x05, delivery_id ‖ request_digest )
```

`request_digest` is the digest of the EXACT protocol bytes prepared
for the partner (delivery-semantics §4.2) ; its definition never
absorbs routing information — the delivery reference carries that.
Two ports of one connector receiving one envelope with identical
bytes therefore derive two distinct calls.

Wiring rule : the **same source port wired twice to the same input port is a
compile error** (a duplicate edge is always a typo — and it would defeat the
recipe).

- **`inbound_seq`** is committed with ingress, monotonically for a source
  provenance. Simulation and single-process runs derive roots from the same
  path/sequence recipe. Each fresh in-memory journal starts its own sequence;
  a diagnostic recording supplies its original facts during replay.
  Durable sequence allocation and multi-host replica-aware roots require the
  M2/cluster contract. M1 adds no hidden clock or random boot salt.

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
a completed `run --record` can be restored by `replay` without touching the
world. That diagnostic artifact is not a crash-safe WAL: a process failure
can lose facts not exported. Durable execution remains M2.

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
2. **`meta.sys.id` is an integrity handle**, not automatically the partner's
   business idempotency key:
   the key names the **business operation** and stays **stable across
   retries** ([delivery-semantics.md §4.3](delivery-semantics.md) — a
   per-call-varying key is the double-pay trap ; the per-attempt form is
   a separate explicit program choice).
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
- **Loop re-admission** → `Loop.next` constructs, or routing into it selects,
  one next-state envelope. The producer transaction retains a durable
  `CommandProduced` fact naming that exact cause. After a separate
  `CommandConsumed` fact and any logical delay, `l:next` re-admits the state at
  the remembered entry. The run identity derives from the exact opening
  admission reference, not merely `origin_id`; a last permitted Next instead
  parents `LoopExpired` on the resolved host terminal and creates no re-admission. Again retains
  `Loop.state` but uses its routed envelope as the command cause.

### 3.1 Transactions without a port arrival — trigger and parent

Every transaction has a **causal envelope** ; the `l:` trigger namespace covers the
transactions that no port arrival triggers :

| Transaction | `trigger` | `parent_id` (the cause) | Emitter (`owner_path` in the record) |
|---|---|---|---|
| `on start` | `l:start` | the envelope whose admission created the Node instance | the node itself |
| Instance close request | `l:closing` | the envelope whose handler requested `stop`, or the envelope that created the winning lifetime deadline | the closing owner itself |
| `on stop` via the `stop` action | `l:stop` | the envelope being processed when `stop` ran | the node itself |
| `on stop` via lifetime expiry | `l:stop` | **the envelope that created the deadline** — the last admitted envelope (`idle`), creator (`fixed`), or creating admission of the processing-time box (`tumbling`). Always exists; there is no phantom tick | the node itself |
| Inbound Call terminal | `l:reply` | the committed root request that opened the Call | the inbound Connector instance that admitted it |
| Session terminal | `l:exit` | the envelope that opened the logical Session | the Connector instance that owns the Session |
| Loop re-admission | `l:next` | the durable Next/Again command cause selected for this run and iteration | the Loop manager, recorded at its owner path and generation |
| Call budget expiry | `l:timeout` | the request envelope that created the budget | the Connector owner |
| Join-buffer eviction | `l:evict` | the evicted envelope ([execution-model.md §7](execution-model.md)) | the Operator node |
| Recovery resolution of an unknown outbound | `l:recover` | the request envelope whose outcome is unknown ([delivery-semantics.md §4.4](delivery-semantics.md)) | the Connector node |

The manager rows state the principle for **synthesized envelopes**: a runtime
manager is not a Node or spatial endpoint. Its transaction is represented at
the Extension owner's path and generation, with the manager command or
lifecycle trigger as its cause. The recipe has no out-of-graph spatial
emitter, but it does distinguish an Executor transaction from an Extension
manager transaction.
Saga compensation identities are not allocated by this format.

## 4. The version stamp — one field, the manifest does the rest

`meta.sys.graph_version` is a **`u64` sequential build/deploy number** per app.
In M1 it is fixed at 1 for a fresh in-memory run. The recording retains the
validated plan and its semantic hash. A durable deployment **manifest** is
an M2 design, not a file currently maintained by the runtime:

```
manifest[graph_version] = {
  content_hash : blake3 of the canonical .rkg   // integrity — re-hashable, cannot lie
  semver       : deployment release label     // not an extra DSL attribute
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

A durable single-process manifest and multi-host allocation are both deferred.
M1 does not claim that a deployment ledger has been persisted.


## 5. Representation and codecs

```text
envelope record
  meta.sys + meta.app + meta.ext  -> deterministic ccbor
  data                           -> [encoding tag, original/current bytes]
                                    raw | ccbor | arrow-ipc
```

Metadata uses the same canonical value registry as the plan and causal
projection. Avro is an import/export schema projection; it is not an internal
envelope or journal encoding. M1's journal remains in memory; the recording
codec writes complete records without claiming durable execution.

### 5.0 Four planes, never confused

| Plane | Answers | Carried by |
|---|---|---|
| Logical type | what do ports and expressions see? | resolved graph schema |
| Internal encoding | how is this DATA represented? | Raw, Ccbor or ArrowIpc |
| Connector codec | how does a boundary decode/encode the outside? | Contract and codec attributes |
| External content type | what does the outside call these bytes? | `meta.ext.content_type` |

The media type never selects a core decoder. A `json! { … }` literal is a
logical value, not raw JSON text. A structured HTTP response is decoded and
checked at the boundary; it is not parked under raw with guessed access.

### 5.1 Lazy reading and decoder identity

1. An identity pass-through or a route using only metadata does not force
   payload decoding. Encoded bytes are shared immutably.
2. **Raw means external bytes.** A `bytes` reading port always sees
   `Value::Bytes`; a `text` port sees `Value::Text` and rejects malformed
   UTF-8 with a typed fault. No content sniffing selects between them.
   For `é\n`, `size` is three on bytes and two on text.
   One reader's cached interpretation cannot alter another port's reading.
3. **Graph-born logical values use ccbor**, including text and bytes.
   Materialized and encoded ccbor states have the same canonical DATA form.
   The decoder checks consistency, exact consumption and canonical
   re-encoding. Raw bytes that happen to encode a ccbor value remain raw:
   their DATA discriminator is different.
4. An opaque Arrow IPC value stays opaque unless an admitted reader supports
   it. No fallback decoder fabricates a value. Boundary wire codecs such as
   JSON do not allocate competing internal DATA tags.

**Attested decoder registry — M2 design, not implemented in M1.**
A decoder has a canonical textual name, such as `std/ccbor@1` or
`std/raw@1`. The standard namespace belongs to the stdlib; an external
namespace has at least one dot. Its binary id is
`Id::derive(IdKind::Decoder, [canonical_name_utf8])`.
A run's immutable attestation binds that name/id to a conformance fingerprint,
covering the agreed vectors and decoding contract. A missing or mismatched
decoder refuses with `E-DECODER-UNAVAILABLE`; it never guesses an alternative
or consults Content-Type. Different implementations claiming the same name
must pass that same conformance contract.

M1 allocates the Decoder identity kind and specifies this seam only. It has
no dynamic decoder registration, plugin loader, attestation lookup or new
decoder execution path. The active raw/ccbor handling above is built into core.

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
`out = a.data`, a local carrying the witness, a ternary's taken branch
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
journal's opaque logical `SessionId`; it is never a file descriptor, socket
address, negotiated HTTP version, Host or Worker id.

The field is in `meta.ext`, not the closed `meta.sys` fields, because it is
boundary-protocol context that a graph may deliberately preserve or replace
when routing a stored/root-mixed response. Direct replies retain causal
session routing independently of the writable projection. An explicit
`correlate = meta.ext.session_id` asks the Connector to verify and use that
logical token; it never authorizes a physical address lookup from graph data.
The projection is the lowercase 32-hex-character spelling of the reference's
16 bytes. The journal
derives inbound and outbound openings under identity domain `0x06`, and each
graph-to-world message write under `0x07`; the exact framed recipes and causal
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

The envelope carries the **minimal system metadata** ; the **journal** carries the full
graph of events. The journal itself is specified by
[delivery-semantics.md](delivery-semantics.md) §6 — **three CAUSAL record
types** (plus the derived, non-causal SNAPSHOT — rebuildable, excluded from
traces and replay) :
INGRESS (the world enters, `inbound_seq` committed atomically with the
accept), TRANSACTION (the atomic unit : consumed positions, state delta,
emissions, `committed_at`), EGRESS (the graph acts on the world : `open` →
`closed(response | timeout)` or `abandoned`). Supervision decisions ARE transaction
records (`l:*` triggers); declared `Timer` firings ARE Ingress records, while
an observed `now()` instant is key 39 of its firing transaction. Three pieces
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
   (its derived id, its lineage pointers, its content type, its build
   stamp) is on the envelope.
2. **No hidden clock in the envelope or evaluator.** A readable `created_at`
   on every envelope was a per-message ambient clock leak, so it was removed
   (its regulatory purpose moved to the journal's `committed_at`). A handler
   may instead call `now()`: the VM supplies one explicit logical timestamp
   for the whole firing and seals it in that transaction when observable. The
   evaluator never reads the host clock, and the instant is not copied into
   `meta.sys`. A `Timer` produces ordinary one-shot or periodic timestamp
   envelopes only when the program actually needs time events. (The time plane is specified
   with the execution model, #2.)
3. **Determinism is a property of `(graph, journal)`, not of the graph
   alone** (§2.0). Individual transactions are deterministic ; multi-input joins,
   Connector responses and `within` windows are **timed merges** — races whose
   outcomes only the journal captures. The journal records them ; replay
   re-reads them ; the simulation fixture plays the journal's role. Nothing
   nondeterministic is ever *embedded in the envelope* — it is journaled
   beside it.


## 9. Related docs

- [identity.md](identity.md) — canonical registries and causal/unsealed distinction.
- [loop.md](loop.md) — temporal state recursion and journal-owned run context.
- [extensions.md](extensions.md) — Retry and retained attempts.
- [execution-model.md](execution-model.md) — mailbox ordering and dated obligations.
- [delivery-semantics.md](delivery-semantics.md) — journal acceptance and exchanges.
- [grammar.md](grammar.md) — envelope constructors.
