# Design decisions

> One entry per settled decision: what was chosen, why, and what was
> rejected — so a rejected alternative is not re-litigated without new
> evidence. Companion to [`errata.md`](errata.md) (defects found);
> git history carries the full context of each entry.

## D-2026-08-07-1 — The transport artifact is a plan package, not a blob

The compiled artifact is named **plan package** (`rastok_core::plan_package`,
extension `.rkp`): a structured, versioned, validated product of the
compiler — nothing about it is opaque or generic. The API vocabulary is
`encode` / `decode` / `checksum`: an unkeyed BLAKE3 is an integrity
checksum, and `seal` is reserved for a future property that actually
binds trust (a signature). **Rejected:** `blob` (dishonest about
structure), `PlanBinary` (suggests machine code), keeping `seal`/`open`
(overclaims a trust property the format does not have).

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
