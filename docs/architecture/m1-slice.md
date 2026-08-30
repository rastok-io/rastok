# The M1 slice — the public-opening milestone

> Implementation slice — non-normative. Opened 2026-08-06, target 15
> October 2026: the repository goes public with this milestone, and
> its closing demonstration is **a complete flow running without the
> real systems** — compiled, simulated against mocks, golden-traced,
> watched live.

**Current checkpoint (2026-08-27): items 1, 2 and the replacement item 3 are landed, as are
the contract and parser/compiler interludes recorded under item 2.
M1.4 has been normatively rebased by D-2026-08-18-1 before public release.
The executable P0–P3 named-`partition` / `per` draft is not the target source
or package contract; its mechanisms are being realigned behind Node-owned
`instance.key`, independent `lifetime`, Node-private state, and hierarchical
logical addresses. Its valid-program/golden evidence remains evidence, not a
compatibility promise.
R3 executes those hierarchical addresses through producer-sealed instance
references shared by routes, transactions, mailboxes, state, calls, timers,
lifecycle and cursors. The causal trace moved deliberately while semantic
hashes and v9 package bodies remained fixed. R4 executes persistent, idle,
fixed, single-pass, and processing-time tumbling lifetimes, including
generation fences and inner-first Graph closure. R5 executes explicitly bounded
Closing, cancellation and call settlement, redirects new arrivals to the
bounded successor mailbox, then runs `on stop` and reclaims state.
R6 executes nested Graph composition and bounded instance-local joins through
the public Lab. R7 closes the executable slice: removed partition spellings are
recovery-only, the identity ledger is audited, and virtual-shard reassignment is
proved absent from semantic hashes, journal bytes, and replay input. The
reference runtime remains deliberately one-worker; physical cluster leases and
crash recovery are later work.
M1.5 C0 separated Connector/Contract/Driver vocabulary without an identity
move; C1 now publishes stable Connector ids and role requirements in
categorical plan-package v10, with separate compiler `ContractCatalog` and
runtime `DriverRegistry`. C2 now binds every active role before the VM and
routes mixed families without concrete-name dispatch; C3 opens `File`, C4
executes its bounded read/atomic-replace effects behind the generic boundary,
and C5 proves one conformance/Lab/static-composition path for both families.
C6 closes M1.5 with one contributor guide, a data-only pre-load capability
manifest and an active-document/diagram audit, without freezing a plugin ABI;
the [M1.5 exit report](m1.5-exit-report.md) records the final evidence and
identity ledger.
M1.6 H1 is landed under D-2026-08-25-1: categorical package v11 now carries
the closed `Event | Call | Session` lifecycle, and the generic runtime owns
placement-neutral session references, causal session folds and two-phase
bounded inbound admission. HTTP/1.1, HTTP/2 and HTTP/3 remain
physical bindings of the two HTTP contracts, while WebSocket Client/Server
exercise that generic exchange with many messages and one terminal `exit`.
H2 is landed under D-2026-08-25-2: the four exact sheets, codec-selected
ports, portable terminal unions, protocol metadata and family Lab binders now
cross source → validated plan without adopting a product network dependency.
H3 is landed under D-2026-08-26-1: one bounded private pump interface hosts
the statically selectable Hyper/Rustls H1-H2, quiche/BoringSSL H3 and
Tungstenite machinery without entering the core or changing package, semantic
or causal identity. H4 is landed under D-2026-08-26-2: one live HttpClient
Driver consumes the H2 sheet over all three HTTP versions, stops fallback at
request visibility, bounds both body directions, publishes lossless response
metadata and passes the generic Call battery without moving package, semantic
or causal identity. H4.1 is landed under D-2026-08-26-3: categorical package
v12 adds the open `media_type` logical scalar and constructor; typed standard
and custom request headers, response metadata projection, strict OpenAPI
projection and the deterministic project resolver now share the one Rastok
type algebra. H5's generic inbound Call seam is landed under
D-2026-08-26-4/-5: source-aware root admission and one close-once `l:reply`
transaction are executable without an outbound record or moving any existing
identity. H5.1 is landed under D-2026-08-26-6: a real bounded Hyper
HTTP/1.1–2 listener retains request bytes through causal admission, withholds
202 under pressure and closes the same Call exactly once. H5.2 is landed under
D-2026-08-26-7: a graph response is routed by direct lineage to the exact
opening instance before commit, offered through a bounded pending/started
Driver handoff, and consumed by `l:reply` without an OUTBOUND record. Final H5
executes `ack` and `respond` over HTTP/1.1–3 with explicit correlation,
bounded deadlines and close-once races. H6 executes WebSocket Client/Server
over HTTP/1.1 Upgrade and HTTP/2–3 Extended CONNECT through the generic
Session fold. H7 closes static registry assembly, feature/no-std matrices,
independent interop, QEMU evidence and the
[M1.6 exit report](m1.6-exit-report.md). M1.6 is closed.
The original item 3 was replaced before public
release by the logical payload-type and uniform-port foundation in
[`m1.3-type-system-backlog.md`](m1.3-type-system-backlog.md). Item 4 now
continues under the ordered [M1.4 instance backlog](m1.4-instances-backlog.md),
starting at R0/R1 rather than layering Closing over the superseded draft.**

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
joins, keyed Node instances, and the HTTP pair all stand on it — and its
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
   connector edges defer by name until the then-named `Partner` seam (now
   `OutboundConnectorDriver`, D-2026-08-22-1) speaks
   them, one pre-commit choke point judges every emission — including the
   then-synthesized errors — against every pin of its fan-out,
   the builders are physically STRUCTS (tag 17) across all three
   interpreters, and the validator audits pinned edges; `92a4da4`
   fifth pass — Dyn is one rule across checker, validator, and
   runtime, the then-synthesized error emissions take the same pre-commit
   gate as everything else, and a bare `data` relay preserves the
   logical Struct/Map type; `76b220a` sixth pass — the struct
   witness rode the then-named `cel::Value`, so every equivalent spelling
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
   rides evaluation as the then-named `cel::EvalValue::Payload` (now
   `expr::EvalValue::Payload`), a witness that
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
   `json! [1]` and `[1]` are distinct programs. B4 later restored
   scalar JSON behind the unambiguous bang. R6 later lands join execution and
   static presence without a travelling `JoinMetaPlan`: the v9 validator
   derives the verdict from canonical schemas and topology. Still deferred
   WITH their reasons: `Encoding::Json` lazy decode (v1 decodes structured
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
   pin (`array<variant>`, judged at check for mocks
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
   the effective pin into one derivation (the historical M1.2 helper;
   M1.3 B2 has since replaced it with the total resolved port table
   consumed by the checker, plan validator, and machine — a
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
   typed-error deliverable into `ProcessExit`; Node-instance `stop` remains a
   distinct lifecycle implemented by the following instance slice. Core
   `datatype::DataType` and core `value::Value` now own the complete strict
   JSON Schema/Avro/Arrow logical and materialized domains; resolved type
   parameters are not copied into envelopes.
   `expr::EvalValue` is only the transient carrier of the lazy payload witness.
   Core `Value::validate` and `DataType::violation` provide the single
   structural-validation and recursive-conformance doors used by plans,
   metadata, state, ingress and emissions; JSON no longer owns the decimal128
   domain. B2 now resolves every
   used or declared port into a total `Declared`/`Inferred` table on
   `ValidatedExecutionPlan`; the categorical package retains and re-derives it, and no unresolved
   hole becomes `variant`. B3 has renamed the consumer to `expr`, added typed
   deterministic `is`/`as` narrowing and pinned the compatible operator
   oracle. B4 has landed
   the single byte-adjacent `name!` frontend form. B5 has replaced flat
   connector port/error lists with one resolved `PortContract` table,
   materialized `Process.in/out/err/exit`, moved root-sink behavior into plan
   data, added `codec = bytes` and defaulted `err_codec = lines`, and removed
   the travelling `error_set`. B6 has made `Case { ... }` a general typed
   value constructor, inserted structural-union wrapping once in checked HIR,
   and removed the special `ErrEmit` syntax/HIR path: `err` now emits like
   every other port. B7 has removed `TypedError` and the
   success/failure-only reply split: the Outbound Connector Driver returns one ordered
   completion, the VM commits all its ordinary port outputs atomically, and
   `Process(job)` frames `out*`/`err*` before a final typed `exit`. B8 now
   transports the complete canonical type/value/expression registry, exact
   `ProcessExit`, static `number`, and the mandatory total resolved-port
   table. B9 has closed the strict external projection adapters, removed the
   historical v6 type/expression scaffolding, and made narrowing mandatory
   before structural access to `variant`. Its final review also removed the
   hidden Operator runtime-fault producer: `err` exists only when the program
   declares, emits or wires it, while engine faults reach supervision.
   Ordered gates and evidence are in
   the dedicated
   [M1.3 backlog](m1.3-type-system-backlog.md).
4. **Node instances, keys, lifetime, and Closing** — any Graph, Operator, or
   Connector may select a canonical address once at its boundary through
   `instance.key`; absence means singleton. `lifetime` is independent, so
   keyed/unkeyed and persistent/temporal combinations are all expressible.
   Mutable `var` state is private to its declaring Node; Graph `val` context
   remains lexical. The producer journals each hierarchical target address
   plus its reserved generation as one instance reference, and every keyed
   owner may shard independently of its parent. This slice
   implements the `stop` side of M1.3: close admission, boundedly drain,
   cancel and settle open invocations without suppressing their individual
   typed `exit`, then run `on stop` and reclaim. D-2026-08-18-1 is the
   authority; the ordered
   [M1.4 instance backlog](m1.4-instances-backlog.md) realigns the private
   P0–P3 draft; R0–R7 are landed and the executable M1.4 slice is closed.
   Some dynamic owner values and Graph handlers stay named R1 frontend gates,
   while physical cluster leases and crash recovery remain later work rather
   than being claimed by the placement-neutrality proof.
5. **Joins — `on (a, b)` bounded by `within …` / `last N`** —
   **LANDED with M1.4 R6 (2026-08-22)**: correlation windows and the static
   metadata merge of D-2026-08-10-2, inside the owning Operator instance.
   Candidate tuples use declared-port order and FIFO choices; false `where`
   predicates retain inputs, and `within` / `last` publish typed
   `JoinExpired` rather than dropping. Here
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
   two input envelopes, no single "current" one — R6 refuses it and requires
   a named input; never a silent pick. Named payload witnesses remain
   independently lazy, and Registry-free presence validation forces an
   explicit metadata pick before a required downstream read.
6. **M1.5 — connector Drivers and `File`** — the first new real connector
   family on the conformance harness, governed by
   [D-2026-08-22-1](../spec/decisions.md) and the ordered
   [M1.5 backlog](m1.5-connectors-backlog.md). A stable versioned contract ID
   joins a compiler `ContractCatalog` to a physical `DriverRegistry`; one
   pre-journal binder proves every required `InboundConnectorDriver` and
   `OutboundConnectorDriver` role. Static registration is first-class and
   future plugin loaders must feed the same registry. `File` proves one-file
   `read` and atomic `replace`; directory traversal and single-file `watch`
   remain named limits. C3 has frozen and implemented the sheet, portable
   `FileExit`, read budget, path semantics, and invocation-value transport
   without touching the filesystem; C4 supplies the real Driver and proves
   journal-before-filesystem, portable exits, cancel and no-recall replay. C5
   runs Process and File through one registry-backed battery, gives Lab mocks
   the same exact-id/role/protocol binding as real Drivers, and proves
   Process-only, File-only and combined static builds. Git diff size separates
   File work from the one-time binder tax; no elapsed-time evidence exists, so
   the historical half-day claim is not repeated. C6 closes the slice with the
   single [Connector Driver contribution path](connector-drivers.md) and a
   canonical id/role manifest that contains no executable code; static
   registration remains the only shipped loader.
7. **M1.6 — HTTP/1.1-3 and WebSocket boundaries** — four stable contracts,
   two Driver roles and three generic exchange shapes (`Event`, `Call`,
   `Session`). `HttpClient` and
   `HttpServer` keep Call semantics; `WebSocketClient` and
   `WebSocketServer` carry many bidirectional messages under one logical
   session and one typed terminal `exit`. H0–H7 in the
   [ordered backlog](m1.6-http-websocket-backlog.md) land the categorical
   lifecycle, two-phase inbound acceptance, exact sheets, hosted network
   engine, all three HTTP versions, WebSocket Extended CONNECT and the Codex
   gateway proof. H3 now selects the bounded private pump, Hyper/Rustls for
   HTTP/1.1-2, quiche/BoringSSL for HTTP/3 and Tungstenite for messages. The
   real-UDP [quiche spike](../../spikes/quiche-http3/README.md) and
   [h3 + Quinn comparison](../../spikes/h3-quinn-http3/README.md) prove the
   adoption and substitution paths without moving package, semantic or causal
   identity. H4 now supplies the first live network Driver: HttpClient over
   H1/H2/H3 with exact pre-open request bytes, bounded response projection and
   visibility-safe fallback. H4.1 adds typed HTTP schema intent and the
   project-wide OpenAPI generated-source resolver. H5 executes retained Calls
   over all three HTTP versions; H6 executes complete-message Sessions over
   all three WebSocket handshakes; H7 closes static assembly and portability
   evidence. See the [exit report](m1.6-exit-report.md).
8. **M1.7 — the `Loop` NodeAspect** — first land the common Node taxonomy:
   declaration keyword → `node::Kind`, one implicit/explicit
   `node::Type`, then zero or more `node::Aspect` values. `App` is a Graph
   type; `Loop` and `Saga` are aspects; Reducer/Aggregator/Window/Joiner/
   Router/Dispatch/Fanout remain ordinary guide patterns. L0 also makes the
   source owner attribute singular (`instance = { key = ... }`) while
   preserving the historical identity domain. L1 freezes the protocol and L2
   now carries its closed boundary, plan constants and ordinary Graph handlers
   through categorical v14 with Registry-free validation. L3 proves one finite
   progress verdict per completed traversal,
   without privileging `out`: continuation uses one `next`; ordinary outputs
   or `stop` terminate. L4 now executes the immediate persistent root-Graph
   subset through per-root journal-folded runs, exact fuel and `l:next`,
   including crash reconstruction and independent concurrent roots. L5 adds
   dated re-entry and direct-child Call recurrence. L6 adds the first checked
   theorem fragment: a direct signed-integer measure with a positivity guard
   and exact literal decrement, re-derived by the `no_std` core from v14.
   L7 closes the public composition, identity and portability evidence:
   singleton/keyed concurrent runs, ordinary nested Graphs, bounded Process
   retry with multi-frame data, exhaustion and Closing fencing. Unsupported
   nested Loop owners and spatial recovery stay named rather than being
   over-claimed. The
   checker remains fail-closed at every unfinished gate. Ordered
   work and exit criteria live in the
   [M1.7 backlog](m1.7-loop-backlog.md) and
   [exit report](m1.7-exit-report.md).
9. **M1.8 data policy, secrets, observations and cost** — replace the
    never-executable stringly `cost` map with canonical schema side-tables for
    `@sensitive`, `@secret`, `@personal`, `@measure` and `@cost`; late-resolve secret
    references at authorized Driver settings; derive structured logs and an
    exact per-flow `CostReport` from the journal. OpenTelemetry semantic names
    win where they match exactly and `rastok.*` fills only project-specific
    gaps. Export remains optional for M1 closure; Budget enforcement, FX,
    FOCUS and OpenCost are later consumers. See the ordered
    [M1.8 backlog](m1.8-observability-backlog.md) and the non-normative
    [Budget draft](budget-governance-draft.md). A0–A5 are closed:
    categorical v15 transports every local policy table and independently
    derives effective policy through canonical operations; presentation
    redacts before formatting. Personal data remains independent of
    confidentiality and carries its GDPR legal/identifier flags. Exact
    HttpClient boot references now bind through real and scripted Drivers;
    personal data requires an attested run-scoped profile and raw secret
    payloads remain fail-closed. A5 retains typed transaction/Call/Session
    correlations beside the journal and projects optional JSON logs through a
    bounded never-block lane; payloads are omitted unless explicitly requested
    and then redacted before formatting.
10. **`--watch`** — the HTML+SSE mini-visualization, strictly a
    journal subscriber; the observation plane stays outside the
    core by construction.
11. **Usage review and the example gallery** — continuous, seeded by
    the benchmark's twenty reference solutions, closed by the
    October demo flow: one realistic end-to-end graph (ingest →
    decide → act → journal) running entirely against mocks.

## Minimal evidence

- each lifted deferral: the `E-CHECK-DEFERRED` site deleted, the
  spec section referenced from the change, scenario coverage in the
  Lab, and — where bytes move — a deliberately re-blessed golden
  with its `decisions.md` entry;
- the whole gate green throughout, semantic-hash moves pinned;
- `HttpClient` and `HttpServer` each landing on the conformance path already
  proved by Process/File, with reviewable change-size evidence; no elapsed-time
  claim without an actual time log;
- the gallery running green under `rastok simulate`, the demo flow
  among it;
- the benchmark re-run cheap by construction: tasks and referees
  extended alongside each language lift, prompt card v3 issued at
  the end of the milestone (v1 and the already-frozen v2 stay
  unchanged for the stability measurement).

## Explicit non-goals

No durable journal on disk (the format is designed, not built), no
Saga manager, no `compensable`/`irreversible` execution, no
`Process` service mode, no distribution, no cluster, no distributed
registry or plugin loader (the compiler's local `ContractCatalog` and the
runtime's local `DriverRegistry` are already landed), no TLS commitment
before its decision entry. The parallelism
decisions of [`parallelism-notes.md`](parallelism-notes.md) stay a
decision list — M1 remains one worker.
