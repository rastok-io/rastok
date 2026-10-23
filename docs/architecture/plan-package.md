# The plan package — the transportable, verifiable ExecutionPlan

> **Current package format 23, diagnostic recording 4.**
> This is the only readable format. The [lifecycle identity inventory](lifecycle-registry-identity.md)
> records the trigger-text movement and the version-only package updates.
> The Node-table representation and field allocation are unchanged.
> There is no compatibility reader or converter. The registry remains
> extend-only; changing the public version label remains a pre-publication task.

The package transports a validated plan, not compiler internals or Driver state.
One Node table retains the lexical Graph tree, instance policies and Extensions.
Each Node contains a Graph, Operator or Connector body; only Operator bodies
contain variables and handlers. The App is the root Graph at index zero.
Composing routes through a passive Graph never erases its ownership or lifecycle.

Raw declarations and the independently resolved port table travel together.
Validation re-derives types, annotations, routes, Dispatch, dispositions and
Extension contracts without a hosted catalogue. Driver libraries and deployment
secrets do not enter the package.

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
        │ 1 : format_version   uint      (23)                 │
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
hash or version refuses exactly like a tampered body. Every covered field is
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
   unverified constructor, no mutable access): every Node index in bounds,
   every referenced port declared with the right direction, one root Graph
   and a parent-before-child lexical tree, every route owned by a Graph,
   every handler contained in its
   Operator, activation on a typed input/join/lifecycle, emission on an output
   or a validated manager command, paths and
   names unique, every state write resolvable. Well-shaped bytes naming impossible structure (an
   endpoint naming absent Node 999) refuse typed — and `Execution::new`
   accepts only the validated type, so no raw plan reaches it by
   any path. The compiler's lowering earns the same type once.
2. **Domain canonicity, audited by the same type** — every collection
   the plan documents as canonically ordered actually is: Nodes ordered by
   nature (Graph, Operator, Connector), then path, with a preceding Graph parent;
   Node port tables sorted and unique with
   untyped conventional rows omitted (a typed `in`/`out`/`err` row is a
   real source constraint), reference routes by their canonical encoding,
   each Operator's handlers by activation key and complete canonical Dispatch,
   with exactly one handler per activation; immutable bindings, vars and
   attributes sorted. Case routes, dispositions, productibility and Loop
   entry/terminal/control/effect proofs are independently re-derived. Immutable
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
budget (`canonical_cbor::MAX_NESTING`, 256, root at level 0) refuses as a typed
error, never a stack overflow. Expression and type budgets are distinct and
smaller: `expr::MAX_EXPRESSION_DEPTH` and `datatype::MAX_TYPE_NESTING` are both
128. The parser bounds its recursion and the structural expression tree at
that expression limit; plan validation independently rechecks expressions
before recursive type checking. The reader bound is the format's transport
contract, not an offset from another format or a particular container layout.
It is backed by the [embedded stack qualification](node-realignment-qualification.md).
The transport budget cuts both ways:
`ValidatedExecutionPlan` refuses any plan whose body would nest
beyond it (`NestingExceeded`, measured without recursion), and
the compiler surfaces that refusal as the user diagnostic
`E-PLAN-DEPTH` — so what `encode` accepts, `decode` reads back, with
no gap at the boundary.

The package re-derives expression/Dispatch types, the exact seven built-in
Connector Contracts (including the two Process modes), port tables, routed narrowing, composition, productibility,
Extensions, Loop control/measure and Retry safety evidence without the hosted
ContractCatalog. Unknown contracts or unsupported execution profiles refuse.
Checksums do not substitute for these proofs. Refusals are typed and total;
there is no silent migration.

Data policy applies the same rule to data policy without consulting the compiler
catalogue: every local table is reconstructed, resolved against its adjacent
logical type and checked against its exact supported interpretation and
flat argument schema. Measurement ids and units are ordinary validated text,
not entries in a closed monetary or telemetry registry. A raw
port row is a declaration; resolved-port policy is a separately transported
effective claim. A3 follows the canonical operation graph and requires that
claim to equal its independent derivation exactly. It never rewrites the raw
row or recovers meaning from payload strings.

## Body tables (format 23)

The body is one canonical CBOR value encoding the `ExecutionPlan`.
Compound values use fixed-shape arrays with a leading numeric
discriminant; maps use fixed unsigned keys; absent optional = omitted
key. `tagged_value` is the registry of
[identity.md §3](../spec/identity.md) — the same sub-form the golden
traces seal, shared code and shared bytes. Signed and unsigned integers have
distinct tags, as do raw bytes and ccbor DATA. The surrounding resolved
`DataType` supplies width, scale and closed field contracts; validation checks
conformance before encoding.

```text
plan       := {5: app_schema?, 6: ext_schema?, 7: [asset_bytes*]?,
               8: resolved_ports, 12: [node+]}
                                                  // 5/6 are travelling zone schemas:
                                                  //   [[field (text), type]*], sorted, prefix-free
                                                  //   (D-2026-08-10-1); absent = empty; the
                                                  //   standard ext leaves of envelope.md
                                                  //   §5.2–5.4 are implicit
                                                  // 7 is raw asset content in digest order
                                                  // 8 is mandatory and re-derived at decode
                                                  // 12 is the only declaration table

node       := {1: path (text), 2: parent_node_index | null,
               3: [port_decl*], 4: instance, 5: [owner_val*],
               6: [extension_spec*], 7: node_body}
node_body  := [1, [route_rule*]]                   — Graph reference wiring
            | [2, [var*], [handler*]]              — Operator
            | [3, spec]                          — Connector
              // Order is (Graph, Operator, Connector), then path. The App
              // Graph is Node zero, with the unique empty path and no parent.
              // Every other parent is a preceding Graph and the direct lexical
              // path prefix. Every Node owns its ports, including the App.
              // Only the Operator body can contain variables and handlers.
              // Key 6 is a canonical Extension list; App occurs once on the
              // root Graph, Loop on an admissible owner, no implicit order.

extension_spec := [1]                            — App
                | [2, loop_spec]                — Loop
                | [3, retry_spec]               — Retry
retry_spec  := {1: attempts (plan_constant), 2: [case (text)+],
                3: delay?, 4: entry (text), 5: terminal (text)}
              // Positive total attempts; strictly sorted unique terminal cases;
              // resolved ports, idempotence and bounded completion independently
              // checked against the exact outbound Call Contract. No manager port.
loop_spec   := {1: fuel?, 2: delay?, 3: proof?,
                4: entry (text), 5: [terminal (text)*],
                6: [exhaustion_port (text), complete_type]?}
              // At least one of fuel/proof is mandatory.
              // Key 6 resolves an existing host terminal, never a contributed
              // port. Absence requires proof-only impossible exhaustion.
              // Its type and port are independently re-derived/checked;
              // the type participates in structural and wire-depth budgets.
fuel        := plan_constant
delay       := [1, plan_constant]                 — fixed(after)
             | [2, plan_constant, plan_constant] — exponential(first, ceiling)
proof       := {1: measure_tree, 2: invariant_tree?,
                3: [positive_decrement (uint)*]}
              // Decrements are a sorted multiset. Validation derives the
              // same evidence from the host's checked control graph and compares exactly;
              // this map is never trusted as a compiler claim.
plan_constant := [tree, tagged_value]
              // Lowering substitutes every lexical `val`, so tree is closed
              // except for indexed assets. Validation re-evaluates the value
              // with no name environment; a surviving free name refuses
              // instead of introducing a second package spelling. Fuel is a
              // positive integer; delay fields are positive durations and
              // ceiling is no smaller than first.

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
              // HIR identity. All rows above are executable.
closing    := [drain_for_ns (int), settle_for_ns (int)]
              // Both durations are positive. Absence is not a hidden default;
              // validation requires the row on a finite owner whose subtree
              // contains Process. This optional key preserves all existing
              // owner bytes when no closing policy is written.

owner_val  := {1: name (text), 2: evaluation_site, 3: tree,
               4: tagged_value?, 5: annotation_table?}
              // Field 4 is present exactly at Plan. Validation derives the
              // earliest site from lexical dependencies, rejects cycles and
              // shadowing, re-evaluates Plan trees (including assets), and
              // requires exact equality with the transported reduction.
              // Invocation refuses for owner vals; invocation-local vals
              // remain handler steps.
spec       := {1: contract_id (text),
               2: behavior (uint: 1 deterministic · 2 probabilistic),
               3: [[name (text), attr]*], 4: connector_roles,
               5: connector_exchange, 6: idempotent (plan_constant)?}
              // contract_id is canonical <namespace>/<name>@<major>
              // connector_roles := uint: 1 inbound · 2 outbound ·
              //                            3 inbound+outbound
              // connector_exchange := uint: 1 event · 2 call · 3 session
              // std/ is the standard namespace; other namespaces contain a dot.
              // Key 6 is a boolean PlanConstant on admitted outbound Call
              // profiles, absent on Sessions and inbound profiles. Its reduction
              // must equal the effective Contract attribute; never copied to CallOpen.
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
port_decl  := [name (text), mode, type?, annotation_table?]
mode       := uint: 1 input · 2 output
              // Every productive output has an explicit disposition.
              — the third element is the PINNED type (same encoding
              as var types), present exactly when the source
              annotates: an unannotated port keeps its two-element
              form in the BODY. The effective type is independently reconstructed below.
              — validation derives a separate total
              ResolvedPortTable (effective type + Declared/Inferred origin)
              from these rows, connector pins and graph constraints; an
              unresolved variable refuses and never becomes variant
              — every Connector contract row is present and typed;
              Operator/Graph rows may still be source constraints; their
              mandatory effective type is carried by resolved_ports
              — annotation tables are appended only when non-empty. If an inferred raw
              port owns policy, element 3 is explicit null so element 4 is
              unambiguous; its effective type remains mandatory below.
resolved_ports := [boundary_resolved, [node_resolved*]]
                                                   // one row per Node, including root
                                                   // boundary_resolved equals root's row;
                                                   // both are checked against derivation
resolved_port := [name (text), direction, type, origin, annotation_table?]
direction    := uint: 1 input · 2 output
origin       := uint: 1 declared · 2 inferred
              // rows use the same canonical (name,direction) order as the
              // derived table; node row count equals the Node table. Validation
              // requires its policy table to equal the raw re-derived row.
var        := {1: name (text), 2: type, 3: init (tagged_value)?,
               4: annotation_table?}

annotation_table := [[schema_path, annotation_set]*]
schema_path := [path_segment*]                   // empty array = root
path_segment := [1, field (text)]                — struct field
              | [2]                              — array element
              | [3]                              — map key
              | [4]                              — map value
              | [5, branch (uint)]               — canonical structural branch
              | [6, case (text)]                 — tagged-union case
annotation_set := [data_annotation+]
data_annotation := [1, sensitivity]
                 | [2]
                 | [3, personal_flags]
                 | [4, annotation_binding, definition (text), argument_type, tagged_value]
                   // definition and complete typed struct arguments
annotation_binding := uint: 1 descriptive metadata; 2 numerical measure;
                      // unsupported tags/profiles refuse; binding 2 requires
                      // std/measure@3, exactly { id: text, unit: text },
                      // and complete typed constants.
sensitivity := uint: 1 internal · 2 confidential · 3 restricted
personal_flags := uint: bit 0 special-category · bit 1 criminal-offence
                         · bit 2 subject identifier; all other bits refuse
              // Tables sort paths structurally and sets by consumer identity,
              // then declaring names for descriptive applications. Every
              // numerical measure shares @measure's per-path uniqueness.
              // Decode reconstructs the closed domain, then validation resolves
              // every path against the adjacent type and rechecks its target,
              // numerical target and unit consistency per id. Declared
              // applications recheck flat argument types, complete constants,
              // declaring identity and authority; their types/values count
              // against the package-depth budget. Empty tables
              // have no wire spelling at an owning schema site.

endpoint   := [node_index (uint), port (text)]
              // The containing Graph determines the boundary face: its own
              // input is a source and output a destination; a child's input
              // is a destination and output a source. App is Node zero, never null.

handler    := {2: activation, 3: join_guard (tree)?, 4: dispatch}
              // Containment in an Operator body supplies ownership.
activation := [1, port (text)]
            | [2, lifecycle]
            | [3, [port (text)+], within_ns (signed) | null,
                  last (positive uint32) | null]
              // Join ports retain declared order, >=2 distinct.
              // Key 3 is an admission guard on a join only. A single-port
              // where is already OpenMatch, with a no-emission fallback.
dispatch   := [1, [step*]]                         — Linear
            | [2, [pure_let*], tree, [[case_set, dispatch]*]] — ClosedMatch
            | [3, [pure_let*], [[tree, dispatch]*], dispatch] — OpenMatch + fallback
case_set   := [case_name (text)+]                  — canonical, disjoint
              // Closed arms cover the source domain; open arms keep source
              // order. Decisions <=32 deep; combined CBOR levels stay below 256.
lifecycle  := uint: 1 start · 2 stop · 3 next · 4 timeout · 5 evict ·
                    6 recover · 7 closing · 8 reply · 9 exit · 10 retry
              // Only start/stop activate authored lifecycle handlers.
              // `reply` is an internal inbound-Call
              // transaction and is never admitted as a source handler;
              // supervision rows remain internal; only start/stop are DSL handlers
step       := [1, name (text), tree, annotation_table?] — let
            | [2, port (text), emission]          — emit
            | [3, name (text), tree]              — set var
            | [4]                                 — stop current instance
            | [5, [port (text)+], multi_values]   — ordered multi-emission
            | [6, command]                      — own Loop manager command
multi_values := [1, emission]                    — evaluate once, broadcast
              | [2, [emission+]]                 — ordered tuple, exact arity
command    := [1, emission] | [2] | [3]           — Next / Again / Done
emission   := [1] | [2, [write*]] | [3, [write*]] | [4, tree] | [5]
              identity · patch · replace · data · nothing
write      := [target, tree]
target     := [1, [seg (text)*]]                  — data path (empty = whole data)
            | [2, field (text)]                   — meta.app, FLAT dotted field (D-2026-08-10-1)
            | [3, field (text)]                   — meta.ext, flat field
              data path · app field · ext field (D-2026-08-10-1)

route_rule := [endpoint, selection]
selection  := [1, [destination+]]                 — all cases
            | [2, [[case_set, [destination+]]+]]  — closed case partition
destination := [1, endpoint] | [2]                — port / explicit nothing
             | [3, node_index, command_kind]      — manager, never PhysicalEdge
             | [4, graph_index, target_index]     — lexical Graph / exact-instance stop target
command_kind := uint: 1 next · 2 again · 3 done
              // Route rows use canonical encoded-byte order. Destination sets
              // use domain order, independent of source spelling. Same-case
              // duplicate destinations and nothing + real destination refuse.
              // Every rule is stored only in its owning Graph's wiring.
              // For tag 4, graph_index must be that Graph; target_index names
              // itself or one direct child.
              // No instance, key or generation is encoded in the static plan:
              // the producer seals the exact instance::Generation in causal key 42.

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
            | 18                                  — canonical open media_type
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
            | [7, fn (uint), [expr*]]             — call    (1 duration · 2 timestamp · 3 size · 4 decimal ·
                                                  //   5 asset · 6 media_type · 7 has; asset is a decoder-recognized pre-binding
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

`tagged_value` is exactly the registry in
[`identity.md`](../spec/identity.md), including interval tag 21 and both
union selectors under tag 19. A typed
decimal may carry its declared scale, including scale zero; the integer fold
remains a document/constructor reader rule, not a schema-blind package rule.
`media_type` deliberately reuses tagged-value text tag 6; its surrounding
resolved type requires the canonical concrete spelling and prevents arbitrary
text from entering the scalar.
The surrounding `type` in the expression/state/schema context supplies width,
precision, units, fixed sizes and timezone rather than duplicating those
parameters in every materialized value.

From the public reference approval onward, discriminants and keys are extend-only:
append, never renumber.
Anything a future plan carries that these tables cannot express is a
format-version bump, not a creative reuse.

## Derived execution indexes

Validation composes the Graph-owned wiring and builds private route and scope
indexes. They are not additional transported claims. Composed routes preserve
every Graph crossing needed for admission, instances, lifetime and Loop context.
`NodeIndex` selects a declaration, not a causal identity: journal recipes still
use canonical paths and exact instance generations.

Queued `Delivery` values use an execution-local `EdgeIndex`. A `DeliveryRoutes`
table holds the canonical cursor path and arrival port once; preparation and
recovery resolve them before writing or checking journal facts. This table also
covers boundary admissions and temporal readmissions, which are not physical
back-edges. No execution index enters the journal codec or its causal projection.

Nature-specific Node bodies are boxed in memory. The
[qualification report](node-realignment-qualification.md) measures both package
loading and execution/recovery on Cortex-M33; package size alone is not a heap
bound. The in-memory layout is not another field of the package.

## Placement

Both codec directions live in `rastok-core`, beside the tables they share.
The compiler/CLI encodes; embedded boot decodes. LTO can remove the unused
direction from firmware. `canonical_cbor` supplies the byte discipline and
`canonical` the shared value registry, consumed by both the golden projection
and the package codec.

## Verification contract

- **Round trip**: compile → encode → decode ⇒ the decoded plan is
  structurally equal to the original and the header's DECLARED
  semantic hash equals the compilation's. (The hash is computed over
  the HIR and travels with the package; a plan alone cannot
  re-derive it — the round trip checks equality of plan and of
  declared hash, under a checksum-protected association. No party
  proves the hash *belongs* to the plan; the producer declares it.)
- **Reviewed fixture**: the example's package is pinned like a golden
  trace and updated only deliberately, with its reason reviewed.
- **Semantic round trip**: beyond structural equality, the decoded
  plan must EXECUTE identically — the boundary-value matrix
  (`i32::MAX`, `i32::MAX + 1`, `i64::MAX`, negatives) runs on both
  sides of a round trip and compares outputs.
- **Typed refusals**: a bumped version, a corrupted body byte, a
  corrupted declared hash, an out-of-bounds Node index, a non-canonical
  envelope, and a truncated package each fail with their own error —
  tested.
