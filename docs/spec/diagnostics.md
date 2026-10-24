# Diagnostics catalog

> **Status (2026-10-06):** current diagnostics; deferred tool surfaces are labelled explicitly.
> This file is the single registry of tool-output diagnostic codes.

**Scope.** This catalog norms diagnostics emitted by *tools* (compiler,
planner, sim runner) — they never appear in a journal or a golden trace.
Runtime outcome cases (`JoinExpired`, `LoopExpired`,
`SagaRolledBack`, `OutcomeUnknown`, …) are **language material**:
ordinary typed port payloads, hashed through their resolved types and
compared in the causal core. They live in their home specs, not here.

The expression registry includes `media_type(text)` and
`has(object.member)`. New or renamed executable diagnostics enter this
catalog only with focused tests. A deferred design name is not a claim that
the CLI currently emits it; the current core may refuse through an existing
typed capability gate.

`W-DEAD-GRAPH` currently covers nested Graph admission, not unused top-level
Graph definitions. The bounded-join expiry productibility defect found in the
[9 September audit](../architecture/documentation-audit-2026-09-09.md) is fixed:
both `within` and `last` require an explicit disposition for productible
`JoinExpired`. [Checker and hostile-plan regressions](../../crates/rastok/tests/v16_destinations.rs)
cover the refusal, nested boundaries and actual expiry observations.

## Rules

1. **Named codes** : `E-` (error) / `W-` (warning) + UPPER-KEBAB.
   A published code is **never renamed and never reused**.
2. **Stability contract** : stable forever = the code + its structured
   payload fields (what tests and tools key on). Never stable = the
   message text (pedagogy improves freely). The current CLI renders
   text only; when a machine-output mode lands, its stable shape is
   `{code, severity, spans, fields}` rather than the rendering text.
3. **Registration** : any change that introduces a code MUST add its row
   here in the same commit. This file is the collision authority.
4. **Lazy assignment** : many specification rules still describe
   errors and warnings without a named code. Each receives its code
   **when its check is implemented**, in the same change as its negative
   test. Until then, the prose rule remains normative. The deferred names below describe design intent, not allocated package tags.

## Registry

| Code | Severity | Phase | Home spec | Meaning (informative) |
|---|---|---|---|---|
| E-LAB-DUP | error | resolve | [grammar.md](grammar.md) | **deferred:** more than one logical lab for the same App |
| E-LAB-NO-APP | error | resolve | [grammar.md](grammar.md) | a lab whose name binds to no `: App` graph in the unit |
| E-SIM-TARGET | error | resolve | [simulation-semantics.md](simulation-semantics.md) | `simulation … on <graph>` targeting a non-App graph (deferred capability) |
| E-EXPERIMENT-PROFILE | error | resolve | [grammar.md](grammar.md) | **deferred:** experiment without exactly one constant `profile` |
| E-SAGA-DURABILITY | error | plan | [effects-and-sagas.md](effects-and-sagas.md) | **deferred:** a Saga whose effective durability resolves to `none` |
| E-COMPENSATE-KEY | error | plan | [effects-and-sagas.md](effects-and-sagas.md) | **deferred:** compensation without a stable business key |
| E-REPLAY-VERSION-MISMATCH | error | replay | [identity.md](identity.md) | **deferred:** journal segment hash ≠ loaded graph hash ; replay refused |
| E-DUP-KEY | error | check | [identity.md](identity.md) | a STATIC map/struct literal with two keys that canonicalize equal; a runtime collision is instead a typed language error, not a tool diagnostic; field `detail` |
| E-UNION-OVERLAP | error | check | [identity.md](identity.md) | a structural union (`A \| B`) with overlapping or identical branches (`int \| int`, `int \| long`, `number \| long`) — no unique branch; a tagged union may deliberately distinguish equal-shaped cases by name; field `detail` |
| E-LEX-CHAR | error | parse | [grammar.md](grammar.md) | a character outside the lexical grammar; field `character` |
| E-LEX-UNTERMINATED-STRING | error | parse | [grammar.md](grammar.md) | a string literal that never closes |
| E-LEX-UNTERMINATED-COMMENT | error | parse | [grammar.md](grammar.md) | a block comment that never closes |
| E-LEX-UNTERMINATED-BLOCK | error | parse | [grammar.md](grammar.md) | a raw guest block whose braces never balance |
| E-PARSE-EXPECTED | error | parse | [grammar.md](grammar.md) | a token breaking the expected production; fields `expected`, `found` |
| E-JSON-VALUE | error | parse | [grammar.md](grammar.md) | a `json!` region the house reader refuses; field `detail` (JSON is not the Rastok expression grammar — its faults carry their own code) |
| E-PARSE-DEFERRED | error | parse | [grammar.md](grammar.md) | syntax admitted by the grammar but outside the subset this build implements; field `construct` |
| E-APP-COUNT | error | resolve | [grammar.md](grammar.md) | a deployed unit without exactly one `: App` graph; field `count` |
| E-DUP-NODE | error | resolve | [grammar.md](grammar.md) | a node name declared twice in one graph; field `name` |
| E-RESOLVE-UNKNOWN | error | resolve | [grammar.md](grammar.md) | a name resolving to no declaration; field `name` |
| E-PORT-UNKNOWN | error | check | [grammar.md](grammar.md) | a port the addressed node does not expose; field `port` |
| E-META-SYS-READONLY | error | check | [envelope.md](envelope.md) | a write reaching the read-only `meta.sys` namespace (§6: there is no write) |
| E-EXPR | error | check | [grammar.md](grammar.md) | a Rastok expression fault, adapted from the isolated expression compiler; field `detail` |
| E-CHECK-DEFERRED | error | check | [grammar.md](grammar.md) | grammar admitted but outside the executable subset of this build; field `construct` |
| E-DATA-ANNOTATION | error | check | [observability.md](observability.md) | a data annotation is unknown, duplicated, malformed, dynamically parameterized, missing its explicit measurement id/unit, uses retired cost arguments, or is illegal at its resolved canonical target; fields `annotation`, `path`, `detail` |
| E-NODE-ANNOTATION | error | check | [extensions.md](extensions.md) | stable code for an unknown, duplicated, or incompatible Node annotation; a Connector annotation names one exact Contract plus a canonical Extension list; fields `name`, `detail` |
| E-EXTENSION-CONFLICT | error | check, plan | [extensions.md](extensions.md) | two Extensions have incompatible same-owner contributions; C2 checks the unadmitted Loop + Retry combination before transport; fields `owner`, `left`, `right`, `detail` |
| E-RETRY-HOST | error | check | [Retry contract](retry.md#2-source-and-admission) | Retry or plan-owned Call idempotence on a non-Call/unknown Contract or inadmissible owner/profile; field `detail` |
| E-RETRY | error | check | [Retry contract](retry.md#2-source-and-admission) | absent, duplicate, misplaced or unknown policy attribute, invalid positive bound/delay, non-boolean guarantee or contradictory File guarantee; field `detail`; expression/constant faults retain `E-EXPR` |
| E-RETRY-CASES | error | check | [Retry contract](retry.md#2-source-and-admission) | nonliteral case selection, unknown terminal, duplicate, predicate or text instead of a terminal name; field `detail` |
| E-RETRY-EFFECT | error | check | [Retry contract](retry.md#21-repetition-safety) | selected outcome_unknown/after_effect case without the original operation's idempotence guarantee; fields `owner`, `case`, `visibility`, `idempotent`, `detail`; the historical `effect` attribute supplies no guarantee here |
| E-INSTANCE | error | check | [instances.md](instances.md) | a malformed or repeated `instance`/`lifetime` attribute, a superseded `instances`/`partition`/`per` spelling, or an instance key without one total canonical type; field `detail`; post-solver type variants additionally carry `name` |
| E-INSTANCE-COMMAND | error | check | [instances.md](instances.md#73-explicit-closure-commands) | a closure command used as a source/intermediate endpoint, a target other than the wiring Graph or one direct child, or an actual emitter outside that Graph's instance context after boundary composition |
| E-LOOP | error | check | [loop.md](loop.md) | a malformed, duplicate, misplaced, removed, or non-finite `Loop` attribute/contract; field `detail` |
| E-LOOP-CONTROL | error | check | [loop.md](loop.md) | one reachable application path lacks exactly one `Next`, `Again`, `Done`, or `Stopped` verdict, or can select several; fields `owner`, `path`, `detail` |
| E-LOOP-HOST | error | check, plan | [loop.md](loop.md) | Loop is attached to a profile that is not a finite Operator, Graph, or outbound Call application; fields `owner`, `profile`, `detail` |
| E-LOOP-ENTRY | error | check, plan | [loop.md](loop.md) | no unique recurrent input can be derived or the authored `Loop.entry` is not that input; fields `owner`, `entry`, `detail` |
| E-LOOP-TERMINAL | error | check, plan | [loop.md](loop.md) | the natural terminal set is ambiguous, contradictory or ill-typed, or is authored for a Connector Call; fields `owner`, `port`, `detail` |
| E-LOOP-CONTEXT | error | check, plan | [loop.md](loop.md) | **deferred as a tool code:** a route, join or activation can mix Loop runs or let uncorrelated work vote in a run; the current Execution uses this name in its runtime join-context refusal, not a structured compiler diagnostic; planned fields `owner`, `path`, `detail` |
| E-LOOP-CARDINALITY | error | check, plan | [loop.md](loop.md) | a possibly repeated frame can select more than one manager command for one application; fields `owner`, `port`, `detail` |
| W-LOOP-REPEAT | warning | check | [C1 algebra](../architecture/retry-c1-algebra.md#41-required-c2-loop-warning) | authored `Loop.again` may repeat a visible Call without an original-plan idempotence guarantee, including successful sibling Calls; not emitted for `Loop.next` or a wholly BeforeEffect path; no identity contribution |
| E-CONNECTOR-UNAVAILABLE | error | check | [stdlib.md](stdlib.md) | a Connector Contract is unknown, only declared without an executable sheet, absent from the compiler catalogue, or has an invalid semantic id; field `type`. Physical Driver availability is checked separately at deployment; Lab simulation does not require one |
| E-CONTRACT-ATTR | error | check | [stdlib.md](stdlib.md) | an attribute violating its connector contract sheet; fields `attribute`, `detail` |
| E-CONTRACT-REQUIRED | error | check | [stdlib.md](stdlib.md) | a mandatory contract attribute left undeclared; field `attribute` |
| E-DUP-BINDING | error | check | [grammar.md](grammar.md) | a binding name shadowing a visible binding or a reserved handler name; field `name` |
| E-NAME-CLASH | error | resolve | [grammar.md](grammar.md) | a Graph child and one of its boundary ports occupy the same local name; fields `owner`, `name` |
| E-HANDLER-DUPLICATE | error | check | [grammar.md](grammar.md) | an Operator declares a second handler for the same trigger; fields `node`, `trigger` |
| E-GRAPH-HANDLER | error | check | [extensions.md](extensions.md) | Graph application code must move into an Operator child; no compatibility execution path remains |
| E-CONNECTOR-HANDLER | error | check | [extensions.md](extensions.md) | a Connector transition belongs to its Contract and Driver events, never a DSL handler; field `node` |
| E-GRAPH-STATE | error | check | [extensions.md](extensions.md) | Graph-private `var` state must move into an Operator child; fields `owner`, `name` |
| E-CONNECTOR-STATE | error | check | [extensions.md](extensions.md) | author-declared `var` state is not part of the Connector's Contract; fields `node`, `name` |
| E-DISPATCH-POSITION | error | check | [grammar.md](grammar.md) | a match follows a mutation/emission/control statement, an open fallback is misplaced, or a handler repeats or misplaces `where` |
| E-DISPATCH-DEPTH | error | parse, check | [grammar.md](grammar.md) | more than 32 nested match decisions; the desugared single-port `where` counts as one; fields `depth`, `budget` |
| E-MULTI-EMIT-DUPLICATE | error | check | [grammar.md](grammar.md) | an atomic multi-emission repeats an output target |
| E-MULTI-EMIT-ARITY | error | check | [grammar.md](grammar.md) | tuple-valued multi-emission has different target and value counts |
| E-WIRING-MIGRATION | error | check | [grammar.md](grammar.md) | removed `when`, dot-prefixed boundary wiring, a former `Loop.expired` port, or a non-port destination used to continue a path chain; field `construct` |
| E-PORT-DIRECTION | error | check | [grammar.md](grammar.md) | a port used against its direction — edge, handler, or emission side; fields `port`, `direction` |
| E-GRAPH-CYCLE | error | check | [grammar.md](grammar.md) | a wiring cycle among nodes — edges form a DAG; field `node` |
| E-PORT-CONFLICT | error | check | [grammar.md](grammar.md) | a port name declared or inferred with conflicting roles, redeclared, or an unsupported Connector port declaration; field `port`. An existing output refinement is admitted only where its Contract/codec explicitly supports typed JSON deserialization |
| E-META-ZONE | error | check | [envelope.md](envelope.md) | a metadata path outside the three zones (`sys`/`app`/`ext`), outside a zone's vocabulary, a dynamic `meta[…]` access, or a Lab write into the application's zone (D-2026-08-10-1); field `path` |
| E-META-TYPE | error | check | [envelope.md](envelope.md) | one `meta.app` field written with two incompatible types — both sites carried as spans; field `field` |
| E-META-PREFIX | error | check | [envelope.md](envelope.md) | two flat keys where one prefixes the other (`audit` vs `audit.origin`) — the nested projection cannot hold both; field `field` |
| E-ASSET | error | check | [grammar.md](grammar.md) | an `asset(…)` expression that cannot resolve at build — missing file, App-root escape, per-asset (8 MiB) or total (32 MiB) budget breach, a Lab position, wrong arity, or a non-literal text argument (D-2026-08-10-5); field `path` (empty when the call itself is malformed) |
| E-DUP-TYPE | error | resolve | [grammar.md](grammar.md) | a named type declared twice in the unit (D-2026-08-10-8); field `name` |
| E-TYPE-RECURSIVE | error | check | [grammar.md](grammar.md) | a named type reaching itself through its own expansion — recursion refuses in v1, the faulty reference underlined (D-2026-08-10-8); field `name` |
| E-TYPE-DUP | error | check | [grammar.md](grammar.md) | a duplicate field or symbol inside one type declaration — the duplicate is underlined (D-2026-08-10-8); fields `name`, `field` |
| E-TYPE-INVALID | error | check | [types.md](types.md) | a source type or inferred value violates a canonical `DataType` constructor invariant (invalid map key, decimal parameter, table row, or duplicate member); field `detail` |
| E-TYPE-UNRESOLVED | error | check | [types.md](types.md) | an open source constraint reaches a position that requires one closed effective type (for example `struct { .. }` or an empty collection without contextual element type); field `type` |
| E-CONTENT-TYPE | error | check | [decisions.md](decisions.md) | a content-typed byte form (`base64!(ct)`, `asset(path, ct)`) outside whole-payload position — a nested field has no `meta.ext` of its own (D-2026-08-10-5); field `tag` |
| E-TYPE-BUDGET | error | check | [grammar.md](grammar.md) | a type expansion beyond a structural budget — depth (128), nodes (4096), width (256), or alias links followed (256) — with the MEASURED value in the payload (D-2026-08-10-8); fields `name` + `depth`/`nodes`/`count`/`aliases` |
| E-NAME-RESERVED | error | resolve | [grammar.md](grammar.md) | a reserved word in a binding position: `nothing` or `true`/`false`/`null`; a node named `meta`/`data`/`sys`/`app`/`ext`/`instance`; a binding named `data`/`meta`/`instance`; or a user `type` spelling a builtin type. Frontend names are ordinary identifiers; only their adjacent `!` has structural meaning. One naming policy, refusal at the declaration; field `name` |
| E-BRACE-MIXED | error | check | [grammar.md](grammar.md) | a braced value mixing ordinary data fields with reserved envelope roots (`data`, `meta` — `mime_type` left the list with D-2026-08-10-4) — wrap the data in `data = { … }`; never a silent reinterpretation (D-2026-08-09-1); field `field` |
| E-PORT-TYPE | error | check | [grammar.md](grammar.md) | a typed destination cannot accept or uniquely wrap its value: an unsolved port variable, incompatible edge/emission/ingress/state/case-field requirement, zero or several contextual union branches, a dynamic `variant` without narrowing, or an implicit conversion between distinct union types; fields `port` (destination name), `expected`, `found`. Since M1.3 B2, validation owns a total resolved port table and never substitutes `variant` for an unsolved variable. |
| E-DUP-EDGE | error | check | [envelope.md](envelope.md) | the same source endpoint wired twice to the same target, compared after desugaring — the deliveries would derive one call ref; field `edge` |
| E-MATCH-NOT-CLOSED | error | check | [grammar.md](grammar.md) | a closed edge `match` scrutinizes a type other than a closed tagged union, enum or bool; fields `source`, `type` |
| E-MATCH-OVERLAP | error | check | [grammar.md](grammar.md) | two closed arms cover the same case; fields `source`, `case` |
| E-MATCH-NONEXHAUSTIVE | error | check | [grammar.md](grammar.md) | a closed match leaves a productible case uncovered; fields `source`, `case` |
| E-MATCH-DUPLICATE | error | check | [grammar.md](grammar.md) | one source port has more than one edge `match`; field `source` |
| E-ROUTE-DUPLICATE | error | check | [grammar.md](grammar.md) | route expansion selects the same physical destination twice for one source case; fields `source`, `case`, `destination` |
| E-ROUTE-MIXED | error | check | [grammar.md](grammar.md) | `nothing` and a real destination are selected together for one source case; fields `source`, `case` |
| E-UNWIRED-OUTPUT | error | check, plan | [delivery-semantics.md](delivery-semantics.md) | a productible output or closed case has no explicit disposition, recursively including nested Graph boundaries; fields `node`, `port`, `case` (case empty for a whole port) |
| E-DUP-LAB-IDENTITY | error | resolve | [simulation-semantics.md](simulation-semantics.md) | a duplicated lab identity — simulation, scenario, or mock of one connector at one level — or a repeated scenario origin; fragments merge, homonyms refuse; field `name` |
| E-INTERNAL-PLAN | error | plan | [plan-package.md](../architecture/plan-package.md) | the lowering emitted a plan its own validator refuses — a compiler defect surfaced as a diagnostic, never a panic; field `detail` |
| E-PLAN-DEPTH | error | check, plan | [plan-package.md](../architecture/plan-package.md) | at check, an expression exceeds `expr::MAX_EXPRESSION_DEPTH` (128, also the parser recursion bound, one unit per syntactic level); at plan, the exact serialized depth reaches the independent reader bound `canonical_cbor::MAX_NESTING` (256, root at level 0); field `depth` in both phases |
| W-DEAD-GRAPH | warning | check | [grammar.md](grammar.md) | a nested Graph has no external admission and its output obligations are suspended; field `owner`; unused top-level graph analysis remains deferred |
| W-DEAD-OUTPUT | warning | check | [delivery-semantics.md](delivery-semantics.md) | an author-declared output is proven impossible under the closed source/Contract analysis and therefore needs no destination; field `source`; implicit contributions proven impossible produce no warning |
| W-DEAD-INPUT | warning | check | [delivery-semantics.md](delivery-semantics.md) | **deferred:** a required input is never fed; a required respond-return half is an error instead; fields `node`, `port` |
| W-DURABILITY-WEAKENED | warning | plan | [delivery-semantics.md](delivery-semantics.md) | **deferred:** a Node or nested Graph explicitly selects a weaker scope than its closest inherited value |
