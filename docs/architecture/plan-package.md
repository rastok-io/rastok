# The plan package — the transportable, verifiable ExecutionPlan

> Format note — the bytes follow this document, never precede it.
> Specified 2026-08-06 (M0.5 item 4), after the delivery-reference
> work froze the vocabulary the plan carries; **revised to format v2
> on 2026-08-07** after the adversarial review refuted v1 on three
> counts (an ambiguous value tag, a checksum that did not cover the
> declared identity, and a decoder that validated shape but not
> structure — errata E-2026-08-07-1, decisions D-2026-08-07-2/-3);
> **revised to format v3 on 2026-08-08** when the M1 multiport lift
> widened `port_decl` with the optional pinned type
> (D-2026-08-08-3) — a new format is a new version, so the typed
> table did not ride under a v2 header; **revised to format v4 on
> 2026-08-09** when the braced-form reform admitted the EMPTY struct
> literal (tag 17) into expression constants (D-2026-08-09-2) — an
> extension of the expressible values is a version bump, exactly as
> this document's own rule demands.
> The package is a TRANSPORT format: versioned and free to evolve by
> version bump; none of v1, v2, or v3 was ever published and no
> decoder reads them — each refuses by its number.
> **Format v10 (D-2026-08-22-1, C1 identity checkpoint, landed
> 2026-08-22)** retains the complete v9 owner/instance/type/value/expression
> transport and makes Connector identity categorical. Each Connector spec
> carries a validated `<namespace>/<name>@<major>` id and its required runtime
> role set; source names such as `Process` no longer travel. Registry-free
> validation checks canonical ids and the built-in Process id/role contract.
> V9 is categorically refused rather than guessed or migrated.
>
> **Format v9 (D-2026-08-18-1, R2 transport tranche, landed
> 2026-08-21)** refuses the unreleased v8 named-partition draft and adds the
> categorical lexical-owner table. The root App Graph, nested Graphs, and
> executable Nodes retain parent-linked ownership; each Node points back to
> its owner and each edge carries its lexical Graph scope. Owners transport
> the exact `instances.key` tree and inferred type, independent lifetime, and
> phased immutable values. Connector attributes may likewise transport a
> checked boot/instance/invocation expression; a plan-shaping dynamic row
> refuses without consulting the compiler ContractCatalog.
>
> Owner and execution tables are deliberately distinct. A Graph owns lexical
> context and lifecycle but is not an executable VM Node; an Operator or
> Connector has one row in each domain. Validation proves the reciprocal
> references and the parent tree before either table is observable. This
> preserves runtime `Slot` stability while preventing DAG flattening from
> erasing Graph ownership. V9 did not smuggle in the later journal move: R3
> has since landed mandatory `(address, generation)` route/transaction
> references as their own declared causal-trace change. That change touches
> no package byte. The R2 tranche moved package containers and checksums while
> unchanged source kept its semantic hash and causal trace; R3 moves the trace
> while keeping both semantic hash and package fixed.
>
> **M1.5 C3** is the first public source consumer of an invocation-phase
> Connector row: `File.path`. The existing v10 `AttrSetting::Expr` form
> carries its checked text tree and actual site; Registry-free validation
> re-derives the site and contract ceiling. The evaluated path is call data,
> not a package mutation, so C3 allocates no format tag and moves no existing
> package or semantic identity.
>
> **M1.6 H1 is categorical format v11, not an implicit v10 extension.**
> D-2026-08-25-1 requires every Connector to carry its closed
> `Event | Call | Session` exchange shape and placement-neutral session
> references in causal records. The affected package/hash/trace sets were
> enumerated before implementation. Format v10 is categorically refused; no
> decoder guesses the missing exchange fact.
>
> **M1.6 H2 uses v11 unchanged.** The four network sheets produce ordinary
> Connector ids, roles, exchanges, effective attributes and resolved ports
> through forms already categorical in v11. Standard HTTP/WebSocket
> `meta.ext` leaves are implicit typed vocabulary rather than a serialized
> schema claim. H2 therefore moves no existing package, semantic hash or
> causal trace and adds no format tag.
>
> The lifetime rows are one closed transport and executable registry: v9
> validation admits `persistent`, `idle`, `fixed`, `single_pass`, and
> processing-time `tumbling` on Graph and Node owners. R5 adds the optional
> exact bounded `closing` row and the `stop` step under the same categorical
> v9; sliding/event-time remain named gates. R6 completes the already
> categorical handler registry with join activation tag 3 and makes nested
> Graph composition public. Existing packages do not move: the new activation
> row occurs only in newly executable join programs, while nested Graph source
> already maps to the owner/edge tables v9 was designed to carry.
> R7 changes no package byte: it categorically rechecks that the unpublished
> v8 body remains refused, removes the old partition spellings from executable
> syntax, and proves that the physical virtual-shard-to-Worker lookup is absent
> from the v9 body, semantic identity, journal, and replay input.
> A decoded key type is re-derived and compared; unlike
> compiler lowering, package validation never normalizes the claim.
>
> Format v7 (D-2026-08-14-1, landed 2026-08-15) was the one categorical
> M1.3 B8 publication: the complete canonical
> `DataType` constructor registry, all materialized value tags allocated in
> identity.md, the full expression alphabet (`struct`, tagged constructor,
> `is`, `as`) and the total resolved-port table travel together. A decoder
> rebuilds the effective table from raw constraints and compares it exactly
> with the transported claim; no compiler ContractCatalog or unresolved inference
> variable can enter execution. `Process.exit` therefore carries exact
> `ProcessExit`, and `decimal()` is statically `number`; their former v6
> representation bridges are gone. V6 joins the refused drafts and is never
> migrated silently. Avro branch indexes, Arrow type codes and dense/sparse
> layout remain adapter projection facts mapped explicitly to canonical union
> discriminants, never logical type identity.
> **B9 closure (2026-08-15):** no format change. Strict JSON Schema, Avro and
> Arrow projection happens outside the package and produces only canonical
> `DataType` plus adapter-owned physical selector mappings. The historical v6
> type/expression subset predicates and format refusals are deleted; an
> unbound `asset()` call has its own invariant error. External schemas cannot
> smuggle an alternate type registry into v7 or the categorical M1.4 format.
> **Historical private v6 B5 revision (2026-08-15):** v6 was never published, so
> B5 replaces its current canonical body rather than allocating a transitory
> v7 before the categorical type registry lands. Connector spec key 5
> (`error_set`) is retired; complete resolved Connector port rows travel, and
> each row's mode now includes its unwired policy. A previous v6 body is not
> migrated: validation or canonical re-encoding refuses it. The semantic
> moves and affected goldens are recorded under D-2026-08-14-1. B8 subsequently
> replaced this body with the categorical v7 registry above.
> **Historical private v6 B7 revision (2026-08-15):** Process framing produced unit
> outputs (`lines -> text`, `jsonl -> variant`, `bytes -> bytes`), while
> stderr follows the independently resolved `err_codec`. The in-memory
> completion gate required exact `ProcessExit`; only the raw v6 plan row kept
> `exit: variant` because the v6 type grammar could not transport tagged unions.
> This is one named representation bridge, not the execution contract. The
> plan remained structurally v6, but effective Process port meaning changed its
> canonical body and semantic identity. B8 removed the bridge in the single v7
> type/expression publication.
> Format v5 (D-2026-08-10-9, landed 2026-08-10): the
> three-zone metadata write targets, the travelling `app`/`ext`
> schemas, the content-addressed asset table, expanded named types
> (they never reach the format — expansion is the checker's), and
> the revised numeric canonicalization (D-2026-08-10-7). v4 joined
> the refused drafts. **v5 is FROZEN (second freeze, 2026-08-10)**: the first freeze
> was lifted the same day to settle two review findings — the
> implicit `content_type` (key 6 carries sheet fields only) and the
> recursive value forms (maps compose through the MapLit expression;
> a non-empty struct inside a map is INEXPRESSIBLE in v1 and defers
> by name, so no new expression form was needed). Nothing was ever
> published between the freezes. The proposed travelling `JoinMetaPlan` never
> landed: R6 derives static metadata-presence facts from v9's canonical schema,
> topology, activation, and explicit-write rows, avoiding a duplicate claim.
> Identity stays where it always was — the semantic hash over the
> canonical HIR; the package merely carries a plan and proves it intact.

## Shape

One byte discipline exists in this repository — the deterministic
CBOR of the golden traces (RFC 8949 §4.2 profile: shortest-form
integers, definite lengths, fixed unsigned map keys in table order).
The package speaks it too; `postcard` and friends were rejected for the
canonical roles (non-canonical, implicit schema —
[decisions.md](../spec/decisions.md) D-2026-08-03-1).

The API lives under `plan` — the codec is a capability of the plan,
not a domain of its own: `rastok::plan::encode(&ValidatedExecutionPlan,
&SemanticHash) -> Vec<u8>` and `rastok::plan::decode(&[u8]) ->
Result<(ValidatedExecutionPlan, SemanticHash), DecodeError>`.
`encode` is infallible because structural validity rides in the type;
what `encode` accepts, `decode` reads back — by construction. "Plan
Package" stays the artifact's documentary name (`.rkp`); no public
module carries it.

```text
package := magic "RKPB"                      4 raw bytes, sniffable
           ("Rastok Plan Binary" — the pinned meaning of the magic)
        ┌ canonical CBOR map ─────────────────────────────────┐
        │ 1 : format_version   uint      (10)                 │
        │ 2 : semantic_hash    bytes(32) (the plan's identity)│
        │ 3 : body             bytes     (embedded, see below)│
        │ 4 : checksum         bytes(32) (see recipe below)   │
        └─────────────────────────────────────────────────────┘

checksum = blake3( "rastok.plan-package\0"   fixed domain string
                 ‖ format_version (u32 LE)
                 ‖ semantic_hash  (32 bytes)
                 ‖ body bytes )
```

The checksum covers the **whole association** — a tampered declared
hash or version refuses exactly like a tampered body (v1 covered the
body alone; refuted by counter-example). Every covered field is
fixed-width or last, so fields cannot slide into one another. The
vocabulary is exact: the producer **declares** the semantic identity
(computed over the HIR, which the plan alone cannot reproduce); the
format **protects the association** against later corruption; the
decoder verifies the association and **never re-derives the claim**.
An unkeyed BLAKE3 is an integrity checksum: it proves the package
untampered in transit, not who wrote it — authorship would need a
signature, which this format does not claim. A version outside the
readable set is reported exactly as read, never saturated.

The body is an **embedded byte string**, verified before a byte of
it is parsed. After parsing, the decoder holds the plan to the rest
of the contract, in order:

1. **Structural validation, carried by a type** — the parsed plan
   passes through `ValidatedExecutionPlan` (private field, no
   unverified constructor, no mutable access): every slot in bounds,
   every referenced port declared with the right direction, every Node and
   owner row reciprocally linked, one root Graph and a parent-before-child
   lexical tree, every edge scoped to a Graph, every handler owned by its
   node, activation on an input port, emission on an output port, paths and
   names unique, every state write resolvable. Well-shaped bytes naming impossible structure (an
   edge to slot 999) refuse typed — and the machine's `Vm::boot`
   accepts only the validated type, so no raw plan reaches it by
   any path. The compiler's lowering earns the same type once.
2. **Domain canonicity, audited by the same type** — every collection
   the plan documents as canonically ordered actually is: owners in root-first
   path order with a preceding Graph parent, Node port tables sorted and unique with
   untyped conventional rows omitted (a typed `in`/`out`/`err` row is a
   real source constraint), edges by their endpoint text,
   handlers by (slot, activation key) with one handler per
   activation, immutable bindings, vars and attributes sorted. Immutable
   owner expressions additionally have an acyclic lexical dependency graph,
   one derived earliest phase, and (at Plan) one independently reproduced
   reduction. Attributes are the
   EFFECTIVE sheet (D-2026-08-12-1): the checker injected every
   omitted contract default before lowering, and nothing of the
   metamodel itself (presence, defaults, profiles) is serialized —
   complete Connector port tables are resolved and typed before lowering;
   every expression map literal
   construction-sorted, walked recursively through guards, steps,
   and writes. These orders live in domain arrays the serializer
   reproduces faithfully, so the re-encoding check below cannot see
   them — the type must.
3. **Canonicity, as one total check** — re-encoding the decoded plan
   must reproduce the body byte for byte, and re-encoding the header
   fields must reproduce the envelope. This refuses non-minimal
   integers, reordered or duplicated map keys, and every other
   non-canonical variant at once, instead of chasing them rule by
   rule.

The canonical reader is itself strict and bounded: success means
the bytes were canonical (minimal heads, strictly increasing unique
map keys, the one canonical NaN), and nesting beyond its documented
budget (`canonical_cbor::MAX_NESTING`, 128) refuses as a typed
error, never a stack overflow. The budget cuts both ways:
`ValidatedExecutionPlan` refuses any plan whose body would nest
beyond it (`NestingBudgetExceeded`, measured without recursion), and
the compiler surfaces that refusal as the user diagnostic
`E-PLAN-DEPTH` — so what `encode` accepts, `decode` reads back, with
no gap at the boundary.

Expression typing and arbitrary compiler-ContractCatalog coherence remain the
compiler's job. For executable built-ins, the package validates the resolved
contract facts it must dereference: the active `Process` attributes and exact
four-row port table are checked without a Registry. The package's validation holds the *structural*
invariants the runtime dereferences without checking. Refusals are
typed and total; there is no silent migration — a new format is a
new version, and an old decoder refuses it by name.

## Body tables (format v11)

The body is one canonical CBOR value encoding the `ExecutionPlan`.
Compound values use fixed-shape arrays with a leading numeric
discriminant; maps use fixed unsigned keys; absent optional = omitted
key. `tagged_value` is the registry of
[identity.md §3](../spec/identity.md) — the same sub-form the golden
traces seal, shared code and shared bytes; since 2026-08-07 the Rastok
`uint` domain has its own registry tag 23, and tag 3 is `long` only
(E-2026-08-07-1 — v1 reused tag 3 for both, and a decode could turn
an `int` into a `uint` while keeping the plan's hash). Since v4 an
expression literal may carry the struct tag 17. V7 admits the complete
materialized struct family; the surrounding resolved `DataType` supplies its
closed field contract, and validation checks conformance before encoding.

```text
plan       := {1: [node*], 2: [port_decl*], 3: [edge*], 4: [handler*],
               5: app_schema?, 6: ext_schema?, 7: [asset_bytes*]?,
               8: resolved_ports, 9: [owner*]}
                                                  // 2 is the App boundary
                                                  // 5/6 are travelling zone schemas:
                                                  //   [[field (text), type]*], sorted, prefix-free
                                                  //   (D-2026-08-10-1); absent = empty; the
                                                  //   standard ext leaves of envelope.md
                                                  //   §5.2–5.4 are implicit
                                                  // 7 is raw asset content in digest order
                                                  // 8 is mandatory and re-derived at decode
                                                  // 9 is mandatory, including the root Graph

node       := {1: path (text), 2: node_kind, 3: [port_decl*],
               4: [var*], 5: owner_slot}
node_kind  := 1                                   — operator
            | [2, spec]                           — connector
              // Only Node rows are executable and they retain the established
              // Slot namespace. Validation proves owner_slot names exactly
              // one owner_kind=[2,this_node_slot] with the same path.

owner      := {1: path (text), 2: parent_owner_slot | null,
               3: owner_kind, 4: instance, 5: [owner_val*]}
owner_kind := [1]                                 — graph
            | [2, node_slot]                      — executable Node owner
              // Canonical root-first path order. The root App Graph is the
              // unique empty path, parentless Graph. Every other parent is a
              // preceding Graph owner and the direct lexical path prefix.
              // Graphs retain scope/lifecycle without becoming VM Nodes.

instance   := {1: key (tree)?, 2: lifetime, 3: closing?}
              // key's tree already contains its inferred DataType. Absent =
              // singleton inside the parent instance. Validation checks the
              // expression under every effective admitting input, one total
              // key type EXACTLY equal to the transported tree type, the
              // canonical key domain, metadata schema, and lexical availability.
              // A data-dependent key with no resolved admission input refuses;
              // its transported result type is never trusted as a fallback.
lifetime   := [1]                                 — persistent
            | [2, nanoseconds (int)]              — idle(after), > 0
            | [3, nanoseconds (int)]              — fixed(after), > 0
            | [4]                                 — single_pass
            | [5, nanoseconds (int)]              — tumbling(every), > 0
              // Persistent is explicit in the categorical plan even though
              // omitted and explicitly persistent source spellings share one
              // HIR identity. R4 executes every row above.
closing    := [drain_for_ns (int), settle_for_ns (int)]
              // Both durations are positive. Absence is not a hidden default;
              // validation requires the row on a finite owner whose subtree
              // contains Process. This optional key preserves all existing
              // owner bytes when no closing policy is written.

owner_val  := {1: name (text), 2: evaluation_site, 3: tree,
               4: tagged_value?}
              // Field 4 is present exactly at Plan. Validation derives the
              // earliest site from lexical dependencies, rejects cycles and
              // shadowing, re-evaluates Plan trees (including assets), and
              // requires exact equality with the transported reduction.
              // Invocation refuses for owner vals; invocation-local vals
              // remain handler steps.
spec       := {1: connector_id (text), 2: effect,
               3: behavior (uint: 1 deterministic · 2 probabilistic),
               4: [[name (text), attr]*], 6: connector_roles,
               7: connector_exchange}
              // connector_id is canonical <namespace>/<name>@<major>
              // connector_roles := uint: 1 inbound · 2 outbound ·
              //                            3 inbound+outbound
              // connector_exchange := uint: 1 event · 2 call · 3 session
              // key 5 (error_set) is RETIRED by B5 and never reused
effect     := 0 | [1, class (uint: 1 idempotent · 2 compensable · 3 irreversible)]
attr       := [1, word (text)] | [2, tagged_value]
            | [3, evaluation_site, tree]
              // plan-resolved choices/constants use the first two forms.
              // The third carries an expression explicitly authorized by the
              // contract at the ACTUAL boot, instance, or invocation site;
              // the contract's latest permitted site stays compiler-side,
              // while this selected phase is plan data and is never recovered
              // from the compiler ContractCatalog. This checkpoint admits the row
              // only when the core has an explicit contract mirror; an
              // unknown contract or attribute refuses closed.
              // Form 3 may not claim Plan: a plan result uses form 2.
evaluation_site := uint: 1 plan · 2 boot · 3 instance · 4 invocation
port_decl  := [name (text), mode, type?]
mode       := uint: 1 input/require-wire · 2 output/require-wire ·
                    3 output/root-sink
              // mode 4 (input/root-sink) is decoder-recognized only so a
              // forged body receives a precise typed refusal; encode never emits it
              — the third element is the PINNED type (same encoding
              as var types), present exactly when the source
              annotates: an unannotated port keeps its two-element
              form in the BODY. The claim is scoped: the body bytes
              of an unannotated plan are unchanged, but the PACKAGE
              moved to v3 with this widening (version is checksum
              material), and the semantic hash of any graph with a
              port table moved with the annotation's identity slot —
              both pinned by the gauge fixture (D-2026-08-08-3)
              — since M1.3 B2, validation derives a separate total
              ResolvedPortTable (effective type + Declared/Inferred origin)
              from these rows, connector pins and graph constraints; an
              unresolved variable refuses and never becomes variant
              — since B5, every Connector contract row is present and typed;
              Operator/Graph rows may still be source constraints; their
              mandatory effective type is carried by resolved_ports
resolved_ports := [boundary_resolved, [node_resolved*]]
                                                   // one row per executable
                                                   // Node Slot, not per owner
resolved_port := [name (text), direction, type, origin]
direction    := uint: 1 input · 2 output
origin       := uint: 1 declared · 2 inferred
              // rows use the same canonical (name,direction) order as the
              // derived table; node row count equals the Node table
var        := {1: name (text), 2: type, 3: init (tagged_value)?}

edge       := [scope (owner_slot), endpoint, endpoint | null]
              // scope is the lexical Graph that owns the source edge;
              // null target = «-> nothing». The owner tree + scope derive
              // every Graph boundary entered/left by the route. The compiler
              // may bypass a passive composed-Graph port, but it may not erase
              // that Graph's owner row or key/lifecycle crossing.
endpoint   := [node_slot (uint) | null, port (text)]
              // slot=null denotes the App boundary; otherwise it indexes the
              // executable Node table. Edge scope independently preserves the
              // lexical Graph whose boundary owns the route.

handler    := {1: slot (uint), 2: activation, 3: guard (tree)?, 4: [step*]}
              // slot names the owning executable Operator or Connector
activation := [1, port (text)]
            | [2, lifecycle]
            | [3, [port (text)+], within_ns (signed) | null,
                  last (positive uint32) | null]
              // join ports stay in source-declared order; >=2 and distinct.
              // A bound requires exact typed output `expired`; `where` lives
              // in handler key 3 and no second correlation key is transported.
lifecycle  := uint: 1 start · 2 stop · 3 compensate · 4 next ·
                    5 timeout · 6 evict · 7 recover · 8 closing
              // registry is closed and extend-only; M1.4 instance lifecycle
              // activates start/stop, while aspect/supervision rows remain
              // governed by their own feature gates
step       := [1, name (text), tree]              — let
            | [2, port (text), emission]          — emit
            | [3, name (text), tree]              — set var
            | [4]                                 — stop current instance
emission   := [1] | [2, [write*]] | [3, [write*]] | [4, tree] | [5]
              identity · patch · replace · data · nothing
write      := [target, tree]
target     := [1, [seg (text)*]]                  — data path (empty = whole data)
            | [2, field (text)]                   — meta.app, FLAT dotted field (D-2026-08-10-1)
            | [5, field (text)]                   — meta.ext, flat field
                                                  // 3 (custom namespace) and 4 (mime) are RETIRED
                                                  //   draft tags — extend-only, never reused
              data path · app field · ext field (D-2026-08-10-1)

tree       := [type, expr]
type       := 1                                   — null
            | 2                                   — bool
            | [3, signedness, width]              — integer
            | [4, width]                          — IEEE float
            | [5, precision, scale]               — decimal128
            | 6                                   — exact number
            | 7                                   — text
            | [8, length?]                        — bytes; null = variable
            | temporal
            | 10                                  — uuid
            | [11, [[field (text), type]*]]       — closed struct
            | [12, type, type]                    — map key/value
            | [13, type, length?]                 — array; null = variable
            | [14, [symbol (text)+]]              — enum
            | [15, [type+]]                       — structural union
            | [16, [[case (text), [[field, type]*]]+]] — tagged union
            | 17                                  — variant
signedness := 1 signed · 2 unsigned
width      := uint: integer 8|16|32|64; float 16|32|64
scale      := canonical signed CBOR integer
length?    := null | uint                         // fixed bytes: > 0;
                                                  // fixed array: may be 0
temporal   := [9, 1, date_unit]                   — date
            | [9, 2, time_unit]                   — time
            | [9, 3, time_unit, zone]             — timestamp
            | [9, 4, time_unit]                   — timestamp_ntz
            | [9, 5, time_unit]                   — duration
            | [9, 6, interval_unit]               — interval
date_unit  := 1 day · 2 millisecond
time_unit  := 1 second · 2 millisecond · 3 microsecond · 4 nanosecond
zone       := null | text                         // null is canonical UTC
interval_unit := 1 year_month · 2 day_time · 3 month_day_nano ·
                 4 month_day_millis_unsigned
expr       := [1, tagged_value]                   — literal
            | [2, [seg (text)*]]                  — path
            | [3, unop (uint), expr]              — unary   (1 not · 2 neg)
            | [4, binop (uint), expr, expr]       — binary  (1 mul … 13 or, declaration order)
            | [5, expr, expr, expr]               — ternary
            | [6, expr, expr]                     — index
            | [7, fn (uint), [expr*]]             — call    (1 duration · 2 timestamp · 3 size · 4 decimal;
                                                  //   5 asset is a decoder-recognized pre-binding
                                                  //   form that validation refuses)
            | [8, [expr*]]                        — list literal
            | [9, [[key (text), expr]*]]          — map literal (already construction-sorted)
            | [10, index (uint)]                  — asset reference into body key 7
                                                  //   (D-2026-08-10-5 ; the HASH seals the content
                                                  //   digest, never this index)
            | [11, [[field (text), expr]*]]       — closed struct constructor
            | [12, type, case (text), [[field, expr]*]] — tagged constructor
            | [13, expr, narrow_target]           — is
            | [14, expr, narrow_target]           — as
narrow_target := [1, type]                        — resolved type
               | [1, type, discriminator]         — external discriminator
               | [2, case (text), struct_type]    — tagged case
discriminator := [field (text), [[external (text), case (text)]*]]
struct_type  := type constrained to discriminant 11
```

Every sequence that is semantically unordered is constructor-canonical before
encoding: struct fields, enum symbols, tagged cases and resolved-port rows are
sorted by their declared keys; structural-union branches use the canonical
`DataType` order. Constructors reject duplicates, overlapping structural
branches, empty enums/tagged unions and the published member/node/nesting
budgets. The package reader additionally enforces its byte and CBOR-depth
budgets before constructing a type.

`tagged_value` is exactly the extend-only registry in
[`identity.md`](../spec/identity.md): the M1.4 format retains every v7 B1 family,
including interval tag 24 and both union selectors under tag 21. A typed
decimal may carry its declared scale, including scale zero; the integer fold
remains a document/constructor reader rule, not a schema-blind package rule.
The surrounding `type` in the expression/state/schema context supplies width,
precision, units, fixed sizes and timezone rather than duplicating those
parameters in every materialized value.

All discriminants and keys are extend-only: append, never renumber.
Anything a future plan carries that these tables cannot express is a
format-version bump, not a creative reuse.

## Placement

The codec lives in `rastok-core` — both directions. The brief placed
the encoder std-side; implementing both halves beside the tables they
share is simpler and loses nothing: the *callers* keep the intended
layering (the compiler/CLI encodes, the embedded boot decodes), and
LTO strips the unused half from a firmware. The generic canonical
CBOR machinery moves from `golden` into the kernel for the same
reason — one implementation of the byte discipline, used by the
golden projection (std, evidence) and the package (core, transport).

## Verification contract

- **Round trip**: compile → encode → decode ⇒ the decoded plan is
  structurally equal to the original and the header's DECLARED
  semantic hash equals the compilation's. (The hash is computed over
  the HIR and travels with the package; a plan alone cannot
  re-derive it — the round trip checks equality of plan and of
  declared hash, under a checksum-protected association. No party
  proves the hash *belongs* to the plan; the producer declares it.)
- **Blessed fixture**: the example's package is pinned like a golden
  trace and re-blessed only deliberately.
- **Semantic round trip**: beyond structural equality, the decoded
  plan must EXECUTE identically — the boundary-value matrix
  (`i32::MAX`, `i32::MAX + 1`, `i64::MAX`, negatives) runs on both
  sides of a round trip and compares outputs.
- **Typed refusals**: a bumped version, a corrupted body byte, a
  corrupted declared hash, an out-of-bounds slot, a non-canonical
  envelope, and a truncated package each fail with their own error —
  tested.
