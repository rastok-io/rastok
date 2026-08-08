# Design decisions

> One entry per settled decision: what was chosen, why, and what was
> rejected — so a rejected alternative is not re-litigated without new
> evidence. Companion to [`errata.md`](errata.md) (defects found);
> git history carries the full context of each entry.

## D-2026-08-08-3 — A pinned port type is a public contract

Second multiport lift. A port annotation (`input evt : text`)
resolves through the same type subset as state cells (its deferrals
refuse identically) and becomes part of the port's declaration
everywhere the declaration lives: the handler's `data` takes the
pinned type; an emission, an injection, or an edge whose PRECISE
type disagrees with a pinned end refuses as `E-PORT-TYPE` (`dyn` on
either side is inference territory and passes); the package carries
the type as `port_decl`'s optional third element — absent when
unannotated, so pre-typed-ports packages are byte-identical; and the
annotation is IDENTITY (identity.md: types are sensitive), hashed
with the var-init presence idiom. Unannotated graphs with port
tables gain one absence tag per port in the hash — an identity move
declared here, pinned by no golden. **Rejected:** hashing the
RESOLVED type (inference does not exist yet; annotations-as-written
is the honest contract), and a runtime type check at edges
(static-by-default; the boundary will get its own validation with
the connector sheets).

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
