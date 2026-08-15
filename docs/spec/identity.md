# Identity and canonical form

> **Status: Normative — under review.** This document fixes the
> **semantic identity** of a graph, the
> **allocation of `graph_version`**, and the **canonical byte form** of
> journal records.
>
> The registry below describes executable plan-package v6. The M1.3
> target in D-2026-08-14-1 transports a canonical `DataType` registry,
> complete resolved port types and structural/tagged unions together in
> v7. B2 already requires and derives the complete effective table when a v6
> plan validates or decodes; this registry remains only the current wire
> authority and is not the design constraint for v7.

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

**Sensitive to** : topology (nodes, edges, ports, edge configs),
partitions and `per` bindings ; handlers, the order of statements inside
a handler (`emission_index` = text order) and the order of ports in a
join ; every Rastok expression (hashed as its canonical expression AST) ;
resolved logical types and complete port tables, attributes,
annotations, `variant` clauses ; `var` defaults
(the recovery fold reads them) ; guest-language block bodies (opaque — raw bytes
after common-indentation stripping) and the pinned guest language,
frontend/compiler/lowering profile, ABI and `GuestExecutor` identities.

Rules :

- `env()` / `secret()` hash **as expressions** : the same source has the
  same identity in dev and prod ; values live in the deployment profile,
  never in the identity.
- A future precompiled `GuestProgram` hashes its content digest plus its pinned
  ABI/profile identities in place of a source block. Changing a guest
  toolchain or executor therefore changes the semantic hash and allocates a
  new `graph_version`; Wasm syntax remains out of scope here.
- **No α-equivalence** : node names are `node_path`, hence id material —
  renaming a node is an identity change.
- Identity = canonical-form equality, nothing more ; semantic
  equivalence of different programs is out of scope.

**B5 connector-contract move (2026-08-15).** A resolved Connector's complete
port rows — name, direction, unwired policy and effective logical type — are
canonical HIR identity. `err_codec` is a normal `Defaulted` attribute, so its
omission and `err_codec = lines` are identical after pre-hash injection. The
former `error_set` identity slot is retired rather than retained beside the
typed port model. Consequently Process graphs move once (echo/spike
`a7511135…`, JSONL `4eacb55c…`, compiler probe `f068bcd1…`). The
connector-free gauge semantic hash remains `0d3ad43d…`: B5 only makes its
already-existing implicit Operator runtime-fault row explicit in the package,
which changes transport bytes but not program meaning. These are pre-public v6
checkpoints; v7 later seals the full tagged types without renumbering their
logical registry.

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

**Virtual dated time is NOT a sealed field**, consistently with
simulation-semantics §6.1, which excludes the arming `t` from the
fingerprint. A timeout firing at t = 48 h *is* behavior — but
that behavior manifests as a **record that exists in a position**
(the dated-event firing produces its TRANSACTION), and the absolute
µs value is **derivable from (journal, state)** (delivery-semantics
§4 : every dated obligation is derivable). So virtual time is
recorded as an **annotation** (available to `trace dump`, re-derived
on replay), never a compared byte of the seal. Two runs differ in the
seal because their *records and orders* differ, not because a time
field differs.

**INBOUND** (kind 1 — an envelope entered) — keys `20–24` :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 20 | envelope | map | the spine (keys 1–9) of the entering envelope |
| 21 | connector_path | text | the ingress node's root-relative path (root recipe input, envelope.md §2) |
| 22 | inbound_seq | uint | the atomic sequence for `(connector_path, replica)` — minted by the owning shard's leader ; root recipe input |
| 23 | replica | uint | the logical replica id = `shard_id` ; sealed so the record autonomously reconstructs the root recipe `(connector_path, replica, inbound_seq)` ; the entry is passive, the owning shard's leader mints under this stable, worker-independent identity |
| 24 | ingress_port | text | the graph input the envelope entered — a causal fact : replay re-routes from this port ; **omitted for the default `in`** (absent = omitted), so pre-multiport records are byte-identical (D-2026-08-08-2) |

**TRANSACTION** (kind 2 — one node firing ; the commit packet of
execution-model §1) — keys `30–37` :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 30 | node_path | text | the firing node, root-relative |
| 31 | partition_key | bytes | the instance's canonical key bytes ; omitted for a single-cell node |
| 32 | trigger | text | namespaced `p:<port>` / `l:<lifecycle>` (envelope.md §3 ; closed `l:` registry) |
| 33 | consumed | array | the consumed input **positions** — each a fixed tuple `[edge (text), source_partition (bytes), seq (uint)]` (the cursor key, delivery-semantics §2 — per `(graph-edge × source partition)`, NOT per port : several edges feed one port, so `(port, seq)` is not injective) ; **array sorted by `(edge, source_partition, seq)`** — what leaves the boxes |
| 34 | join_inputs | array | for a join trigger, the presence-completing set (envelope.md §3), each a spine-`id` (bytes 16) ; **sorted by the trigger's port order then id** ; omitted otherwise |
| 35 | state_delta | array | canonical **sorted pair list** `[field_path (text), tagged_value]` of the `var`/partition-field changes `state → state'` (sub-form below ; fields sorted by `field_path`) ; omitted when empty |
| 36 | emissions | array | **in text/`emission_index` order** (execution-model §4) ; each entry a fixed tuple `[port (text), emission_index (uint), envelope (spine 1–9)]` |
| 37 | timers | array | dated obligations armed by this firing — each a fixed tuple `[kind (uint), ref (bytes)]` where `ref` identifies the obligation (call id / join / instance) ; **sorted by `(kind, ref)`** ; **the due µs is NOT sealed** (derivable and re-armed by the fold) |

**OUTBOUND** (kind 3 — one external call's WAL record ;
delivery-semantics §4) — keys `40–46` :

| Key | Field | CBOR type | Notes |
|---|---|---|---|
| 40 | emitting_id | bytes(16) | the id of the envelope that opened the call (the cause) |
| 41 | connector_path | text | the calling node |
| 42 | call_key | bytes | the `idempotency_key` (E-COMPENSATE-KEY) ; omitted only for `compensable` without key |
| 43 | request_digest | bytes(32) | the boundary integrity seal (delivery-semantics §4.2) |
| 44 | status | uint | the SEALED causal states : `1 = open · 2 = closed · 4 = abandoned` ; value `3` is reserved — `discarded` (a late reply after closure) is a NON-causal observability record, never a sealed status |
| 45 | closure | array | present iff status ∈ {closed, abandoned} ; EXACT byteshape fixed BY `status` (`[1,env]` response · `[2]` timeout · `[]` abandoned ; sub-forms below), so `status` and `closure` never disagree |
| 47 | edge | text | the canonical edge of the causing delivery ; with key 40 (`emitting_id`) it reconstructs `delivery_ref` and hence `call_ref` (envelope.md §2) — the record stays autonomous |
| 46 | edge_cursor | array | present only on an inter-shard delivery — the DURABLE LOGICAL cursor `[edge (text), source_partition (bytes), seq (uint)]` (seq never restarts on leader change ; the transport `leader_epoch` fence is separate and NOT sealed) ; a transport cursor, NOT an envelope id, outside every id domain |

Key 44's `status` is the internal state of the current v6 WAL record, not a
graph port and not a payload union. M1.3's conventional `exit` port therefore
does not rename this concept by implication; format v7 separately replaces
the one-envelope closure shape with its typed multi-output completion.

**Normative sub-form encodings.** Every
compound value in the tables above has a fixed shape and, where it is
a tagged union, a **numeric discriminant** — so two implementations
cannot diverge on bytes for the same journal :

- **`tier2` (spine key 7)** = the fixed birth-stamp fields
  (envelope.md §1.2), each present only when born in scope, encoded as
  a canonical sorted list of pairs `[stamp_kind (uint), value]` sorted
  by `stamp_kind`, from the EXACT set :
  `1 = saga_id (bytes 16) · 2 = saga_stack (array of bytes 16) ·
  3 = loop_id (bytes 16) · 4 = loop_stack (array of bytes 16) ·
  5 = iter_count (uint)` (extend-only ; no field renamed or dropped).
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
  | 24 | interval (**reserved for v7; v6 refuses**) | `[months (int), days (int), nanoseconds (int)]` ; the resolved interval `DataType` fixes the admitted component shape (`year_month`, `day_time`, `month_day_nano`, or Avro's unsigned month/day/millisecond profile) |

  The v6 row for tag 21 describes its only executable union form:
  structural `[branch_index, tagged_value]`. The M1.3 logical value algebra
  also has name-stable tagged unions; its in-memory canonical projection is
  `[case_name (text), tagged struct payload]`, deliberately distinguishable
  from the unsigned structural selector. No validated v6 plan can carry that
  form. B8 freezes both selector forms as part of the one v7 registry rather
  than retroactively extending v6.

  The materialized carrier does not duplicate its resolved type parameters.
  Integer/float width, fixed length, decimal precision, temporal unit and
  timestamp timezone come from the plan/schema selected by the record's
  `graph_version`; the value carries only the canonical data and semantic
  discriminants. B8 therefore seals a typed value from `(resolved DataType,
  Value)` while storing that schema context once in the retained plan/manifest,
  not as a full type tree in every envelope. The old v6 `value_tagged` rows stay
  byte-stable for its admitted subset; tags and type constructors unavailable
  to v6 are categorically refused by its validator.

  (Sort keys — struct by `field` UTF-8 ; map by the key's canonical
  `tagged_value` bytes. All temporal logical types share the Arrow
  nanosecond convention — no µs anywhere in the seal.)
- **The `encoding_tag` registry** — EXACTLY the fixed internal
  representations of envelope.md §5, `raw | avro | arrow-ipc`,
  extend-only, never renumbered : `1 = raw (identity bytes, no codec)
  · 2 = avro · 3 = arrow-ipc`. **A JSON payload is `[raw, bytes]` with
  `meta.ext.content_type = application/json`, NOT a separate tag** (the specification
  excludes a JSON tag) ; `json / protobuf / parquet / msgpack` are
  *boundary* codecs a Connector speaks on the wire, never internal
  `data` representations — so two runtimes cannot seal the same JSON as
  `[raw,…]` vs `[json,…]`. `data` (spine key 8) is
  `[encoding_tag (uint), bytes]` — the bytes are opaque, compared
  as-is, never re-encoded.
- **`state_delta` (TX key 35)** = a canonical sorted list of pairs
  `[field_path (text), tagged_value]` sorted by `field_path` — every
  value self-describing via the `type_tag` registry above.
- **`app_meta` values (spine key 9)** : each triple's third element is
  a `tagged_value` (per the registry above).
- **`timers` (TX key 37)** = tuples `[timer_kind (uint), ref (bytes 16)]`,
  `timer_kind` from `1 = call_timeout · 2 = join_within ·
  3 = instance_lifetime · 4 = saga_timeout · 5 = loop_backoff`
  (extend-only), `ref` = the obligation's derived 16-byte identity —
  for a call budget, the `delivery_ref` of the arming delivery
  (envelope.md §2) ; the due µs is never sealed (annotation).
- **`closure` (OUTBOUND key 45) — the EXACT byteshapes.**
  The rule is « one call = exactly one closure ». The SEALED
  causal `status` (key 44) is exactly `1 open · 2 closed · 4 abandoned`
  (value `3 discarded` is NOT a sealed status — see below). Key 45 is
  present **iff `status ∈ {closed, abandoned}`**, and its byteshape is
  fixed :
  - `status = 1 open` → key 45 **absent**.
  - `status = 2 closed`, `outcome = response` → `[1, envelope]` (a
    2-element array : the partner's reply, success OR a typed-err reply).
  - `status = 2 closed`, `outcome = timeout` → `[2]` (a **1-element**
    array — no second element, never `[2, null]` ; `l:timeout` emits
    `TimeoutExpired` as its own TRANSACTION).
  - `status = 4 abandoned` → `[]` (the **empty** array — present but
    zero-length ; the §4.4 triage resolved `OutcomeUnknown`, whose
    envelope is `l:recover`'s TRANSACTION emission). The ONE home of
    `OutcomeUnknown`.
  - `discarded` (a LATE response arriving AFTER closure —
    delivery-semantics §4 / execution-model §6.3, **graph-invisible**) is
    **NOT a sealed OUTBOUND status** : it is a separate
    NON-CAUSAL observability record, never part of the seal. Its payload
    (the late reply, possibly a typed outcome) is a diagnostic
    annotation, so two runs seal identically regardless of a discarded
    late reply. The correlator (runtime) decides it; the adapter never
    returns it as a normal outcome.
- **`consumed` / `join_inputs` / `emissions`** — the fixed tuples and
  imposed array orders stated in their rows above.

**Run-affecting bindings are attested.** The contract
`run = f(graph, journal)` requires that any
deployment binding able to change the run — a `var` default, a
`timeout`/`lifetime`, a partner URL, the tzdata version, a class
policy — be **journaled as a boot attestation record** (its version
or content hash) before first use. Recovery under a differing
attestation is a typed refusal, the `E-REPLAY-VERSION-MISMATCH`
family. Secrets stay out of the journal in clear ; only their
version is attested. This is a control-journal fact, not an
App causal species.

**Collision guarantee.** Two records
differ in bytes **iff** they differ in any causal fact : the kind,
any referenced envelope's spine (incl. the namespaced `app_meta`
triples and `tier2` stamps), the INBOUND `(connector_path, replica,
inbound_seq)` root recipe, the firing node/key/trigger, the consumed
positions, the typed `state_delta`, the ordered emissions, the call
key/digest/status and the discriminated `closure`. **Virtual time is
NOT among them** (it is derivable — an annotation, above) :
two runs differ in the seal because their *records and orders*
differ, never because a time value differs. Conformance vectors (a corpus of
minimally-differing record pairs proven to differ in bytes) are required for
the journal/golden implementation.

**Key table — segment header** : `1 = seq (uint)`,
`2 = semantic_hash (bytes(32))`. Extend-only, same rule.

**Causal core vs annotation ring.** Criterion : *can two correct
executions differ on this field ?* Yes → annotation. No → core.

| Causal core (in the canonical form, compared bytewise) | Annotations (recorded if useful, excluded from the form and the seal) |
|---|---|
| record kind ; full `meta.sys` (id, parent_id, origin_id, role, graph_version) ; port ; `emission_index` ; the causal record fields (consumed set, state delta, ordered emissions, call key/digest/status) ; every typed port payload | **virtual dated time** (derivable — annotation, simulation-semantics §6.1) ; wall-clock (`committed_at`) ; execution durations ; cost / metrics ; host info ; runtime version ; thread ids ; all non-normative diagnostics |
