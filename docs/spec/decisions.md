# Design decisions

> One entry per settled decision: what was chosen, why, and what was
> rejected — so a rejected alternative is not re-litigated without new
> evidence. Companion to [`errata.md`](errata.md) (defects found);
> git history carries the full context of each entry.

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
order or a scheduler order. The lowering emits a static
`JoinMetaPlan`: per-field ops in canonical key order; the runtime
executes a verdict. `content_type` is NOT part of the fusion — it
derives from the payload decision (D-2026-08-10-4). PRESENCE IS
STATIC: past a join, a field whose merge can yield absent (a
conflict is topologically possible, or no input is guaranteed to
carry it) is OPTIONAL — an optional read (`?field`) stays permitted;
a required read, or a required output contract, refuses at check
until an explicit write has chosen the value. An unused conflict is
silent — the refusal sits at the READ or the contract, never at the
merge itself. The runtime's absent → err rule remains as defense in
depth for invalid plans, never the primary check. Reads never
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
`meta.sys.content_type`, no `meta.app.content_type`, and no
`Data::Encoded.content_type`; the `meta.sys` spine drops to FIVE
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
body key 7 — raw blob bytes in FIRST-USE order (deterministic from
the canonical source), digests DERIVED at decode (and thereby
verified) with uniqueness and both budgets re-checked by the
validator. The out-of-body section deferred WITH the embedded
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
`=` and `:` entries refuses. `map { … }` dies — superseding exactly
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
without a Registry, the round-2 doctrine); the `JoinMetaPlan` ops;
the content-addressed asset section (digests and sizes in the body,
bytes outside it); expanded named types; the structural budgets; the
`decimal` canonical tag. It carries NOTHING for history lookups
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

## D-2026-08-07-2 — The CEL uint domain gets registry tag 23

`uint` is a first-class CEL value domain and needs its own identity;
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
