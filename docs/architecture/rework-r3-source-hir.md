# R3 — source/HIR checkpoint before categorical plan v16

> 2026-09-05. Source/HIR implementation of D-2026-09-05-1. The shipped
> package is still v15. This checkpoint does not claim the R4–R8 transport,
> execution, productibility, effect-proof, or recovery gates.

## 1. Result protected

Every newly admitted source construct has a complete typed HIR representation
and semantic identity. A construct which cannot be represented exactly by the
current executable plan fails before that plan is built. In particular, case
routing must not silently turn into unconditional fan-out, and a manager
command must never become a spatial edge.

The source/API ontology is Node / Graph / Executor / Connector / Extension.
Each Connector owns one Contract. `ConnectorId` is now `ContractId`;
`node::Type` is removed; `node::Extension` carries App/Loop and the named future
Saga. No Executor trait or extra Node category is introduced. Existing exact
Contract strings, roles, exchanges and `behavior` remain unchanged.

## 2. Source and HIR admitted

| Surface | Checked result |
|---|---|
| one handler per trigger | duplicate port/lifecycle/join triggers refuse; changing join bounds cannot create a second trigger |
| single-port `where` | the same OpenMatch as its explicit predicate plus empty fallback; same HIR and hash |
| join `where` | retained separately as the pre-consumption admission condition |
| closed Dispatch | typed union/enum/bool domain, disjoint exhaustive cases, canonical arm order and typed narrowing |
| open Dispatch | authored predicate order, first-match interpretation and required final fallback |
| pure prefix | local `val` bindings may precede a dispatch; effects before a match refuse |
| nested decisions | 32 matches per path, including desugared `where`; balanced parse recovery retains following declarations |
| multi-emission | one ordered HIR operation, broadcast evaluated once or positionally paired tuple; duplicate/arity/type checks |
| source set | independent mpsc physical edges, canonically ordered; no join or synchronization |
| destination set | one routed envelope, canonical dispositions; no duplicate emission |
| edge `match` | closed case table, exact tagged value preserved, destinations checked against the narrowed case set |
| additive routes | duplicate selected destinations refuse; `nothing` is exclusive even across declarations |
| Graph namespace | boundary ports and direct children share one namespace; both declaration orders refuse collisions |
| Loop hosts | Operator, Graph and finite outbound Call profiles resolve entry and terminal policy |
| Loop commands | typed `Next`, cause-only `Again` and explicit `Done`; statement/route forms remain outside physical edges |
| `Loop.expired` | a qualified contributed output, resolved from the parent without an ordinary synthetic `expired` port |
| `Loop.state` | typed from the selected entry, inherited through passive Graph scopes and masked by an inner Loop |

Graph/Connector `var`, Connector handlers, Graph handlers outside the explicit
historical Loop path, edge `when`, malformed Loop selectors and misplaced
Dispatch forms have named diagnostics. A Connector's semantic Contract state
is not author-declared DSL state.

The Graph checker resolves its boundary and Loop entry before its children,
while retaining each child's lexical constant environment. This is required
for descendant `Loop.state` typing and does not manufacture Graph code.

## 3. Identity and transport boundary

Conditional HIR streams seal recursive Dispatch (`rastok.dispatch.r1`),
case/manager routing (`rastok.routing.r1`) and command-based Loop policy
(`rastok.loop.r2`). Existing simple programs add none of these streams.

Canonical route identity merges universal destinations into each case row;
source declaration order, arm order, case grouping and destination-set order
do not change it. Case-to-destination mapping and command kind do. A plain
destination set and the equivalent separate fan-out arrows share the existing
physical-edge identity.

Loop identity seals its owner, entry, terminal set, fuel, delay and authored
measure/invariant. Proof evidence is not substituted for the authored
expression and no new certificate is trusted as executable at R3.

The v15 lowering gate refuses case routes, manager commands, destination sets,
new Executor Extensions, handlerless command-based Loop Graphs, non-projectable
Dispatch and multi-emission. Exact linear/one-port-filter projections and
ordinary mpsc source-set expansion may continue through v15 because they have
an exact existing representation. At R3, Lab mock lowering also refused
non-projectable Dispatch/statements rather than losing them. The later R4
execution checkpoint removes the mock decision gate with a shared core
selector and per-leaf completion proof; application-plan transport remains
gated. See [`rework-r4-plan.md`](rework-r4-plan.md).

No package version, Contract major, causal record tag, journal key or semantic
golden is changed here. The one parser-tree snapshot,
`tests/fixtures/minimal.ast`, changes deliberately: `HandlerDecl.body` becomes
`HandlerDecl.dispatch = Linear { statements, span }`, retaining the same
statement and its source locations. This is an AST-shape snapshot, not a
package, semantic hash or causal trace. R4 owns the single categorical bump
and Registry-free validation of all transported forms.

## 4. Regression evidence from implementation

The focused adversarial tests first exposed and then closed these failures:

1. Swapping two closed-route targets could retain the same hash because only
   physical edges were hashed. The case table now participates in identity.
2. New Executor Loop policy was absent from the hash. Changing entry,
   terminals, fuel or delay now moves it; reordering terminals does not.
3. A handlerless Loop Graph could reach v15 lowering and lose its new control
   model. It now refuses before plan construction, even without an explicit
   manager-command route.
4. Graph-private state received a generic parse error; Connector DSL state
   could reach HIR. Both now refuse at their named ownership boundary.
5. A descendant could not resolve the enclosing `Loop.state`. The boundary
   pass and inner-Loop masking now have source-driven tests.
6. Nested Dispatch had no source budget. The parser rejects the first
   over-budget match and the checker also counts the `where` desugaring.

The same suite covers closed-match overlap/missing cases, typed route
narrowing, exclusive `nothing`, source/destination sets, multi-emission order,
one-handler uniqueness, migration messages and the absence of manager edges.

## 5. Repository-contact clarification

The opening EBNF required newline/semicolon between set members while its
accepted examples and D-2026-09-05-1 use `{ sms_gateway archive }`. R3 retains
that whitespace spelling and corrects only the set productions. Match arms
still require their own separators. This clarification is recorded in the
decision; it is not an undocumented parser extension.

## 6. Deliberate staging and next gates

The historical v15 Loop Graph-handler path, dot-prefixed wiring and current
RootSink/Unwired policy remain executable compatibility material for the
existing corpus. They are not v16 semantics. Mixing new Graph Loop entry or
terminal attributes with old Graph handlers refuses. Their coordinated
removal and fixture migration remain R4–R9 work.

In particular, `.` and `.foo` wiring are accepted **without a diagnostic** at
R3. Their named migration refusal is deferred specifically to **R9**, after
the old-sugar/explicit-boundary twins prove identical plan and semantic hash.
The removal statements in the v16 specification describe that target, not a
refusal already implemented in the current binary.

R5 owns whole-graph productibility and recursive missing-output errors; R6
owns execution of Dispatch and case routing; R7 owns commands, LoopContext,
quiescence and spatial RecoveryFold; R8 owns re-derived Loop proofs and the
complete host/effect/cardinality matrix. R3's typed measure expression is not
an R8 termination proof. Edge capacity configuration, bidirectional wiring,
Saga and other still-unsupported forms continue to refuse by name.

Route matches currently require a source pinned as a tagged union, enum or
bool. An unresolved source refuses with `E-MATCH-NOT-CLOSED`; R4–R6 must join
the case-routing constraints to the validated-plan type-inference pass before
claiming inferred source domains. R5 likewise owns narrowing the full declared
case domain to the proven productible cases.

The historical handlerless, several-output Loop test now requires an explicit
`Loop.terminal` set and then reaches the named v16 transport refusal. It no
longer claims that the new Graph manager protocol executes through v15; the
historical Graph-handler tests retain their atomic-output/stop evidence until
their R9 migration.

M1.9/UI remains paused. Its pre-existing work is retained separately in
commit `171c99c`, before the R3 checkpoint. That commit was checked in an
isolated index snapshot without the R3 compiler/core changes; all targets and
features compile, and its 13 JavaScript graph-layout tests pass.

The review preparation splits the checker into private `dispatch`, `routing`,
`loop_control`, and `extensions` modules, with the unchanged test suite in a
separate `tests` module. The main checker drops from approximately 14,800 to
7,840 lines. The split changes no semantic rule, diagnostic or public API;
it gives the upcoming R4–R8 validation work explicit ownership without adding
an abstraction layer.

## 7. Verification

Executed on 2026-09-05:

| Gate | Result |
|---|---|
| focused compiler tests, all features | 200 passed |
| `cargo test --workspace --all-features --no-fail-fast --quiet` | 1,041 passed, 0 failed |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo doc --workspace --no-deps` | clean |
| `rastok-core --no-default-features`, Cortex-M33, Cortex-M3, RISC-V | all three compile |
| Cortex-M33 release build, warnings as errors, unchanged v15 package | `SPIKE_OK` |
| echo, JSONL and gauge packages/hashes/traces | existing fixture checks pass; no re-bless |
| `git diff --check` | clean |

The first broad runs exposed two intended migration points: the handlerless
Loop test and the parser-tree snapshot, documented above. They were updated
to assert the new source contract, not to relax the transport gate.

An independent failure in the live CLI gateway proof was diagnosed from the
child's stderr: its selected ephemeral port was already occupied before bind.
The HTTP/WS tests had a listener guard, but the concurrently starting UI tests
did not. All loopback CLI listener tests now share that existing guard, and a
failed Upgrade reports the child status/stderr and cleans up the child. The
17-test CLI suite passed in the final workspace run and in three additional
consecutive runs. This prevents contention within this harness, not against
unrelated OS processes; no network runtime or UI behavior was changed.

The embedded re-measurement retains exactly 17,844 decode ticks and 2,713
average / 2,966 maximum relay ticks. Heap high-water remains **129,972 B**,
leaving **1,100 B (0.84%)** under the 128 KiB criterion. Linked `.text` plus
`.rodata` is 668,652 + 60,520 = **729,172 B**, 16 B above the M1.8 measurement
and still below 1 MiB. The embedded semantic hash remains
`e50026eeb0b8dc77fe2f5e52de6dfaa33b50cc351211cd87c21081d28aa43709`.
These are emulated measurements, not a new silicon claim.
