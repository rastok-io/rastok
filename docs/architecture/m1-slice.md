# The M1 slice — the public-opening milestone

> Implementation slice — non-normative. Opened 2026-08-06, target 15
> October 2026: the repository goes public with this milestone, and
> its closing demonstration is **a complete flow running without the
> real systems** — compiled, simulated against mocks, golden-traced,
> watched live.

**Current checkpoint (2026-08-14): items 1 and 2 are landed, as are
the contract and parser/compiler interludes recorded under item 2.
The current transport is plan-package v6; valid-program identity for
the spike remains `c75b840d…`. Item 3 has been replaced before public
release by the logical payload-type and uniform-port foundation in
[`m1.3-type-system-backlog.md`](m1.3-type-system-backlog.md).**

## The shape of M1

M0 proved the loop; M0.5 consolidated the engine
([exit state](m0.5-slice.md)). M1 widens the executable subset — and
the discipline is already in place for exactly this: the checker
refuses every not-yet-executable construct **by name**
(`E-CHECK-DEFERRED`), so the milestone is a chosen list of deferrals
lifted one by one. Lifting one deferral means: its frozen-spec
section becomes executable, its tests and scenarios land, its golden
projection evolves only by a deliberate re-bless, and everything not
lifted keeps refusing by name. The semantic hash will move when the
language grows — each move is pinned deliberately, never observed
after the fact.

## Scope, in order

The order is load-bearing: the multiport machine comes first because
joins, `per`, and the HTTP pair all stand on it — and its
delivery-reference prerequisite was sealed in M0.5 precisely so this
step touches `vm.rs` once.

1. **Multiport execution** — handlers on named ports, multiple
   outputs, typed ports; the OUTBOUND key already carries the
   canonical edge, so call identity is ready for it. One transport
   consequence to honor from day one: when a connector contract
   exposes non-default ports, the plan must carry the RESOLVED port
   table — the embedded validator has no `Registry` to consult, so
   everything it checks must travel in the package.
   **LANDED 2026-08-08** — named ingress (`1d5d1a3`: INBOUND key 24,
   D-2026-08-08-2) and pinned port types (`fa861d8`: `port_decl`'s
   optional type, E-PORT-TYPE, D-2026-08-08-3); routing and named
   outputs were already alive, proved by the lift's probes. Held to
   review the same day and re-closed on four findings (`92da490`
   ingress is not observable, `c8459f9` ONE conformance notion over
   every EmitValue form and the physical int bound, `0b0b963` the
   resolved contract table ships from day one with the registry
   seam, `cf5d21c` format v3 with the gauge fixture pinning hash,
   package, and trace; `8b61739` third pass — the checker and the
   machine share ONE data-construction model: the `{…}` shorthand
   builds the struct the grammar defines, builders fold their writes
   in the runtime's order, constructed content is judged at the pin
   at emission-build time, and the Registry validates sheets at the
   door and is never consulted past resolution; `2aa50f4` fourth
   pass — the frontiers: boundary outputs carry their port, named
   connector edges defer by name until the Partner seam speaks
   them, one pre-commit choke point judges every emission — synthesized
   errors included — against every pin of its fan-out,
   the builders are physically STRUCTS (tag 17) across all three
   interpreters, and the validator audits pinned edges; `92a4da4`
   fifth pass — Dyn is one rule across checker, validator, and
   runtime, synthesized error emissions take the same pre-commit
   gate as everything else, and a bare `data` relay preserves the
   logical Struct/Map type; `76b220a` sixth pass — the struct
   witness rides cel::Value itself, so every equivalent spelling
   preserves it; the validator gates unexecutable connector ports by
   capability; the encoded bare relay passes the representation
   through; all four error production paths pinned at the final
   gate; seventh pass — data materializes lazily (an opaque
   payload relays without a forced decode), struct equality is
   BY NAME in both value domains with duplicate fields refused
   typed at the doors, the conformance judgement is structured
   (`ConformanceViolation`), and the maintainer's syntax reform
   landed as D-2026-08-09-1: one braced form classified once,
   `.` for identity, the `envelope`/`struct` value spellings
   removed — same hash, package, and trace on the gauge; eighth
   pass — the data demand comes from the EVALUATOR itself
   (`DataSlot`/`LazyBindings`: a short-circuited branch, a losing
   mock branch, or a statement past the reply never forces an
   opaque decode — the static pre-scan could not express that), a
   nested empty record is a VALUE (the one struct constant a plan
   may carry: the empty literal; the format reads it, the validator
   admits exactly it), struct equality is sorted-field — total,
   reflexive, and symmetric even on door-refused duplicates — in
   both domains and the CEL `==`, a sub-block's name is a
   classification root like a field's first segment, the frozen
   bench card v1 is restored byte-for-byte with the M1 surface on a
   new v2, and the fresh-data variant is named `FreshData`; ninth
   pass — laziness is SEMANTIC, not syntactic: the whole payload
   rides evaluation as `cel::Value::Payload`, a witness that
   carries through `val` bindings and ternaries and materializes
   only where an operation demands structure, so `val copy = data;
   out = copy` ships the representation exactly like `out = data`
   (the syntactic special case is deleted); the envelope's own
   `Data::materialize_with` cell is the cache authority — an
   inspected-then-re-emitted envelope carries its filled cell, once
   per ENVELOPE; the empty-struct literal moved the package to
   FORMAT v4 (D-2026-08-09-2, v3 joins the refused drafts, goldens
   re-blessed, spike on v4 to the same hash); `FieldWrite` dropped
   its dead as-written path). The trio's own contract types join
   the connector-sheet items below.
2. **The consolidated reform, THEN the `jsonl` codec** — the
   metadata/literal/IDL reform (D-2026-08-10-1 … -9) officially
   PRECEDES this item, and **LANDED 2026-08-10** (`50306de` spec
   closure; `58317cf` the three-zone meta model, static grid,
   content-type lifecycle, and the v5 skeleton; `a986553` the
   literal surface — punctuated braces, verbatim text, strict
   base64, the house JSON reader with exact decimal numbers, and
   digest-sealed assets; `10d2d5c` the structural type aliases;
   `4344d55` the v5 tables stated). Two deliberate identity moves,
   both pinned: the five-field spine with `mime` relocated to
   `meta.ext.content_type`, and trees sealing `[type, root]` so
   `json [1]` and `[1]` are distinct programs (scalar `json`
   spellings later died with D-2026-08-13-1). Deferred WITH their
   reasons: `JoinMetaPlan` execution and the static presence
   analysis (they land with the joins slice — reserved format keys
   announced), `Encoding::Json` lazy decode (v1 decodes structured
   bodies eagerly at the frontier), guest meta writes (guests read
   only), the 64 KiB canonical-type-byte budget (with composite
   pins), and the embedded asset-refusal gate (the loader's line —
   the spike's package carries no assets). The `jsonl` codec
   itself **LANDED 2026-08-11** (D-2026-08-11-1): the canonical
   JSON writer joined the house reader in the core
   (`json::write_value` — struct AND map keys sorted bytewise
   because the line is `request_digest` material, decimals in
   plain notation at their sealed scale, doubles in true
   shortest round-trip form; bytes, encoded payloads and
   non-finite doubles refuse typed); the boundary dispatches on
   `RequestCodec` (`codec = jsonl` on the sheet, `prepare_jsonl`
   frames ONE canonical line); the `Process` adapter decodes
   stdout line by line through the house reader into
   `data = array<variant>`, the offending line riding every
   `DecodeError`; the checker's jsonl deferral lifted. Proven
   against REAL children: a decimal round-trips exactly through
   `cat`, multiple lines decode in order, garbage refuses typed,
   an unserializable request refuses BEFORE any open fact, empty
   stdout is the empty array, and the digest is canonical across
   struct spellings. `mode = service` keeps its deferral — the
   stream machinery is its own slice. The same-day review round
   sharpened five edges (amendment on D-2026-08-11-1): `null`
   executes (the envelope domain gained tag 9's value — `json
   null` runs, a `null` line decodes), the reply shape is a real
   pin (`reply_type` = `array<variant>`, judged at check for mocks
   and at every settled outcome — the record-mock false green
   refuses; `lines` stays untyped by NAME, its typing lands with
   the connector sheets), the plan validator holds the codec's
   domain (`UnknownCodec` — no validated plan silently runs as
   `lines`), framing is exact (a lone `\n` refuses; ASCII-only
   line padding), and the decimal128 domain is held at the doors
   with a writer-side budget mirror. The round also applied
   uniform propagation (D-2026-08-10-2) to MACHINE-minted
   envelopes: connector outcomes and typed errors now inherit the
   request's `app`/`ext` zones (content_type struck — the reply's
   payload is fresh), proven by an app-token round trip through a
   REAL `cat` and through a mock. A second same-day round unified
   the effective pin into ONE derivation
   (`boundary::connector_out_pin`, consulted by the checker's edge
   audit and mock judge, the plan validator, and the machine — a
   jsonl-to-`int` edge now refuses at CHECK), scoped the codec rule
   to the Process contract (domain AND presence — the sheet
   requires it), aligned doubles with the decision's own words
   (true shortest round-trip, no forced fraction — an integral
   double's line reads back as `int`, the inbound domain being
   dynamic), swept the dead `codec = "…"` spelling from the docs,
   and added the jsonl golden fixture (`examples/jsonl.rkg`: hash +
   package + Lab trace sealing the canonical line's
   `request_digest`). Unlocks realistic `File`
   and HTTP payloads.
   **Post-M1.2 interlude, LANDED 2026-08-12** (D-2026-08-12-1,
   before item 3): the node-contract METAMODEL and the uniform
   attribute syntax. One rule — every attribute is `name = value`,
   four value families (bare choice word / CEL expression / braced
   literal / foreign literal); configuration sub-blocks
   (`env { … }`) and tagged initializers (`on_crash { … }`,
   `measure { … }`) are dead forms refusing with migration hints, a
   former discriminant now a closed-choice FIELD. The
   `rastok::contract` module owns the metamodel (AttributeSpec with
   `Presence::{Required, Optional, Defaulted}`, `ValueDomain`,
   profiles with `ProfileSupport::Deferred`, sheet validation
   including compiled CEL defaults); `stdlib` keeps the catalogue.
   `Process` resolves `mode` as a profile selector (default `job`;
   `service` known-but-deferred with deferred-only domains), and
   the checker INJECTS every omitted default (`mode = job`,
   `args = []`, `max_output = "8MiB"`, `on_timeout = kill`) into
   the effective HIR before the hash — omission ≡ exact spelling,
   a DECLARED identity move (every Process graph moved — the hashes
   of that day, since superseded by the review rounds' second move:
   echo/spike `d0bc5576…` → `c75b840d…`, jsonl fixture
   `34341263…` → `60f45914…`, probe `c9866b38…` → `20479b25…`;
   gauge, connector-free, pinned immobile as witness of both). The plan
   carries effective attributes only; the adapter's silent
   `8 MiB`/empty-args fallbacks became typed invariant refusals.
   `codec` stays Required (reaffirmed), `AttrDomain::Forbidden`
   died (a field of another profile is ABSENT, the diagnostic names
   the owner), v5 unchanged structurally.
   **Post-M1.2 parser/compiler interlude, LANDED 2026-08-13**
   (D-2026-08-13-1, before item 3): the host front end was returned to
   a conventional forward lexeme stream. `Lexer` owns the only byte
   cursor and token lookahead; `Parser` consumes complete lexemes and
   closed semantic-region results, with no rewind or parser-side byte
   walk. The grammar is left-factored at identifier-led values;
   `json` and `nothing` are unmaskable value words, `json` accepts
   object/array documents only, and implicit yield is gone (`out =`
   states the emission). CEL extent is one shared
   `Complete | Incomplete` computation with typed closer tracking:
   complete values commit atomically, malformed values stop at a safe
   line boundary, and the following statement survives. Host strings
   and CEL strings are line-bounded; `trivia::comment_at` is the one
   comment recognizer. JSON, verbatim `text`/`base64`, and guest bodies
   retain distinct readers; a guest body reaches its future frontend
   as one exact region, while the guest-specific framing decision
   remains explicitly deferred. The exact `decimal("…")` constructor
   extended the plan call alphabet and moved the transport envelope
   from v5 to **v6** without moving semantic identity. Parser recovery
   changes affected invalid programs only; the full valid corpus and
   the embedded spike kept `c75b840d…`.
3. **Logical payload types and uniform typed ports** — one no_std-ready
   `DataType`/`Value` authority for envelopes, ports, state and
   expressions; complete effective port types after inference;
   structural and tagged unions; lossless supported projections from
   JSON Schema, Avro and Arrow; and `IDENT!` frontends (`json!`, `jq!`,
   `sql!`, …). Connector sheets become parametric typed port contracts.
   Boundary completion may carry several ordinary port outputs, so
   `Process.err` is exact stderr while `Process.exit` carries the tagged
   terminal outcome of each invocation. This generalizes the original
   typed-error deliverable into `ProcessExit`; partition `stop` remains a
   distinct lifecycle implemented by the following `per` slice. Core
   `datatype::DataType` already owns the executable type subset without an
   identity move; the remaining `cel::Value`, `error_set`, `TypedError` and
   success/failure-only reply split are removed by the ordered M1.3 gates,
   not promoted as an intermediate model. Ordered gates and evidence are in
   the dedicated
   [M1.3 backlog](m1.3-type-system-backlog.md).
4. **`per` — the partition surface** — keyed nodes and graphs on the
   flat two-concept model; scheduler keys over the proven multiport
   routing. This slice implements the `stop` side of the M1.3 scope
   contract: stop admission, then boundedly drain or cancel every open
   invocation without suppressing its individual `exit`.
5. **Joins — `on (a, b)` bounded by `within …` / `last N`** —
   correlation windows and the static metadata merge of
   D-2026-08-10-2; the last language lift of the milestone. Here
   `last N` is the existing count bound. There is deliberately no
   journal-querying `last(…)` operator:
   values that must survive are copied explicitly into `meta.app`.
   Conflicting parent values are selected by an explicit write from
   `a.meta…` / `b.meta…`; otherwise CommonOrAbsent drops them, and a
   later required read forces the author to choose. Reads alone never
   propagate metadata.
   Identity note, decided
   with the handler canonicalization: the join's activation KEY must
   preserve its declared port order — `on (a, b)` and `on (b, a)`
   are different identities (identity.md makes port order inside a
   join sensitive), while the order BETWEEN handlers stays
   insensitive and canonically sorted by that key. Identity-form
   note (D-2026-08-09-1): under a join, `out = .` is AMBIGUOUS —
   two input envelopes, no single "current" one — so the joins
   slice must refuse it until a normative base rule names the
   re-emitted envelope; never a silent pick.
6. **`File` connector** — the first new adapter on the conformance
   harness: the half-day claim gets its first real measurement, with
   no network in the way. OPENS WITH the global binder decision
   (D-2026-08-12-1, review rounds): today's world is single-Partner
   (`Process::try_new` refuses any callable connector it does not
   serve), and a second adapter needs the `BoundExecutionPlan`
   router. Its obligations are BY ROLE, not only by call: an
   outbound CALLER binds to exactly one Partner, an inbound SOURCE
   (an `HttpServer`) binds to exactly one ingress provider, and a
   both-roles connector owes both — bind → VM → journal, so nothing
   opens against a world that cannot execute it. Design to
   arbitrate at this item's start.
7. **`HttpClient`** — outbound calls under declared budgets, on the
   harness. Decision point before code: hand-rolled bounded HTTP/1.1
   versus a vetted dependency, and whether TLS enters M1 at all (the
   October demo runs mocked; a `decisions.md` entry either way).
8. **`HttpServer`, ack mode** — the inbound side: ingress authority,
   submission sequence, backpressure at the boundary.
9. **The Loop aspect** — the bounded-iteration primitive, alive on a
   graph.
10. **Cost counters** — the `cost` map (stdlib §6.4) accumulated
    from the journal; a subscriber, not a core dependency.
11. **`--watch`** — the HTML+SSE mini-visualization, strictly a
    journal subscriber; the observation plane stays outside the
    core by construction.
12. **Usage review and the example gallery** — continuous, seeded by
    the benchmark's twenty reference solutions, closed by the
    October demo flow: one realistic end-to-end graph (ingest →
    decide → act → journal) running entirely against mocks.

## Minimal evidence

- each lifted deferral: the `E-CHECK-DEFERRED` site deleted, the
  spec section referenced from the change, scenario coverage in the
  Lab, and — where bytes move — a deliberately re-blessed golden
  with its `decisions.md` entry;
- the whole gate green throughout, semantic-hash moves pinned;
- `File`, `HttpClient`, `HttpServer` each landing with a conformance
  profile — three data points on the half-day claim;
- the gallery running green under `rastok simulate`, the demo flow
  among it;
- the benchmark re-run cheap by construction: tasks and referees
  extended alongside each language lift, prompt card v3 issued at
  the end of the milestone (v1 and the already-frozen v2 stay
  unchanged for the stability measurement).

## Explicit non-goals

No durable journal on disk (the format is designed, not built), no
Saga manager, no `compensable`/`irreversible` execution, no
`Process` service mode, no distribution, no cluster, no runtime or
distributed registry (the compiler's contract `Registry` is already
landed), no TLS commitment before its decision entry. The parallelism
decisions of [`parallelism-notes.md`](parallelism-notes.md) stay a
decision list — M1 remains one worker.
