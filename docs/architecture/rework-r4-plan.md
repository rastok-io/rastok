# R4 — categorical plan and independent validation

> **Status (2026-09-05): open, decision-execution checkpoint.** R3 is
> committed separately from the paused M1.9 UI. The encoded package remains
> **v15**; no v16 package, journal key, Contract major or golden is published
> by this checkpoint. R4–R9 are not closed.

## 1. Invariant protected

A closed match table is not a proof merely because the compiler calls it
exhaustive. Its case sets must be independently checked against the resolved
source type, without the hosted ContractCatalog, before its selections can
drive execution.

This is the common closure obligation of handler `Dispatch` and `RouteRule`.
Their bodies and destinations have different obligations; those do not change
which cases constitute a partition.

## 2. First implemented seam

`rastok_core::plan::validate_case_partition` now verifies:

- a bool, enum or tagged-union source domain, derived from the actual type;
- nonempty case sets, with strictly canonical order inside and between arms;
- membership of every selected case in that domain;
- disjointness within and across arms;
- exact coverage, with the first canonical missing case named.

The verifier does not normalize an invalid table, expand an implicit fallback,
or trust a claimed case count. Its working inventory is bounded by the source
type domain, not by the incoming arm table. Source `_` has already become its
exact remainder at this boundary.

The existing hosted domain helper moves to core, and both source consumers
call the independent verifier on their normalized result. Source errors retain
their existing precise diagnostics. A normalized table which contradicts the
core proof is a compiler defect, reported as `E-INTERNAL-PLAN`; it is never
silently converted to a permissive route.

This is not yet a decoded-v16-plan validation claim. The verifier takes an
already resolved type; R4's full plan validator must independently establish
that type and the surrounding expressions, references and destinations.

## 3. Shared Dispatch and local expression proof

Core now owns the checked `Dispatch`, arm, ordered statement, multi-emission,
envelope-builder and Loop-command algebra. HIR re-exports those types instead
of maintaining a second tree. The source parser and core verifier share the
32-decision depth bound. This does not replace the executable v15 handler
`guard + body` yet: lowering still accepts only its exact existing projection
and refuses the rest before execution.

`rastok_core::plan::validate_dispatch` is called on each checked handler. It
independently re-derives:

- expressions against the supplied immutable signature, including local
  bindings and every emission/multi-emission/Next value;
- exact decision-expression types, canonical expression structure and depth
  before any recursive type walk;
- closed case partitions and selected-union scope, without payload unwrapping;
- positive `is` facts within an open arm only: no sibling or fallback receives
  another arm's facts or local bindings;
- pure match prefixes, no local shadowing/forward references, and compatible
  annotation tables beside local types;
- at least two distinct multi-emission targets and exact tuple arity, without
  sorting authored emission order.

Constructors are checked against their contextual shape, field by field. This
includes media-type literals, widened numeric fields and heterogeneous maps or
arrays whose member type is a structural union. A typed constructor is not
mistaken for an untyped expression, nor accepted just because its root claims
the expected type. Failures name an exact structural path, for example
`dispatch.arms[0].linear[0].value`.

The independent expression re-derivation also exposed a pre-existing hole:
`is`/`as` checked a case's name but not its transported payload type. A forged
`Ready { count: text }` target could claim the existing `Ready { count: int }`
case. The shared expression checker now verifies both against the source
union. The regression failed on the old implementation before the fix; valid
source trees and their identities are unchanged.

**Boundary of this proof:** its environment is an input, not a claim of resolved
port types. It preserves source inference placeholders (`variant` predicates)
without claiming they are statically boolean. Full v16 validation must derive
that environment, discharge inference, bind assets and validate activation,
state/output targets, metadata writes, policy propagation and Loop control and
effects. Closed-match narrowing currently covers the source subset's
`match data`; this checkpoint does not introduce arbitrary-path narrowing.

## 4. Shared routing and additive-disposition proof

Core now owns `RouteRule`, selections, arms, endpoints and manager-command
destinations. HIR re-exports that same algebra. The separate hosted conflict
implementation and its arm/intersection helpers are removed; source diagnostics
translate the core's typed conflict into the existing code, span and case.
The v15 single-edge compatibility helper is also removed from the model: its
admission condition belongs at lowering, where it is now checked directly.

`rastok_core::plan::validate_routing` checks the complete normalized routing
inventory of each Graph, not just isolated match arms:

- destination sets are nonempty, strictly canonical and duplicate-free;
- closed partitions come from the looked-up source type, never route claims;
- a source has at most one closed match;
- universal and case selections are additive without selecting a destination
  twice for the same value;
- `nothing` remains exclusive across declarations, including against manager
  commands, irrespective of declaration order;
- different cases may select the same destination without inventing another
  physical edge; commands remain a distinct destination variant, never a port
  recognized by its spelling.

The verifier neither repairs a malformed set nor merges away evidence of a
conflict. Failures locate the selection or arm, such as
`routes[0].selection.arms[1].destinations`. The checker calls this proof after
endpoint materialization in every Graph. Ordinary source mistakes keep their
named source diagnostics; a contradictory normalized inventory is
`E-INTERNAL-PLAN`.

**Boundary of this proof:** source types are supplied explicitly; unconditional
routes need no closed domain and may still carry inference placeholders. Full
v16 validation must establish endpoint existence and direction, destination
types/narrowing, lexical scope, capacities, productibility and manager
ownership/context. This checkpoint does not turn source routing into an
executable v15 approximation or introduce a second spatial-edge authority.

## 5. Contract-derived Call guarantees

Core now derives `CallSemantics` from the exact Contract identity, direction,
exchange lifecycle and resolved profile. The closed inventories cover Process
job, File read/replace and HttpClient. They specify the sole request input,
exactly-once terminal output, frame production and conservative terminal
visibility from R1. Unknown Contracts, other majors and inadmissible profiles
receive no inferred guarantees. Construction is private; a serialized claim
cannot manufacture a more permissive profile.

The source checker uses these facts for Loop host admission instead of a
Process-specific service exception. For direct routes from a Call output to
its own Loop command, the shared proof checks:

- streamed Process frames cannot establish an at-most-one verdict guarantee,
  even with an idempotent effect: `E-LOOP-CARDINALITY`;
- `Next` and `Again` after possible visibility require `none` or `idempotent`:
  `E-LOOP-EFFECT`; compensation alone is not repeat safety;
- a `before_effect` terminal case permits repetition, while `Done` does not
  repeat the effect;
- File read and HttpClient produce a single response only on `Completed`;
  File replace produces no response frame.

These inventories describe the existing `@1` outcomes without changing those
contracts or their transport. The six `@2` moves still belong to the single
v16 transition. The new source forms still refuse at packaging; the global
execution gate for compensable/irreversible effects is not relaxed.

**Boundary of this proof:** this is direct Call-to-own-manager validation,
not whole-application Loop control. Full v16 validation must also establish
exact run context, finite timeouts, frame/exit command uniqueness, selected
value types, intermediate routes and the effects of a repeated Graph body.
No manager execution or ContractViolation runtime gate is claimed here.

## 6. One envelope-construction algebra

Checked Dispatch and the executable plan now carry the same `Emission` and
ordered `FieldWrite` types. Source checking, expression/type/policy proofs,
Loop measure verification, CBOR transport and runtime construction consume
that one model. The duplicate Dispatch-only emission/write types and the
proof's copy-conversion are removed, not kept as compatibility wrappers.
Lowering only substitutes lexical constants inside the shared expressions.

The five forms remain Identity, Patch, FreshData, Data and Nothing. FreshData
names the existing fresh-data builder uniformly; it keeps the same wire tag
and semantics, including scalar whole-data writes, payload witnesses and
explicit metadata overrides. Field order remains authored order, never a
canonical sort. This consolidation changes Rust representation names, not
source spelling, plan bytes, semantic hashes or causal records.

The categorical v16 gate remains at lowering. Sharing the emission leaves
does not admit recursive Dispatch, multi-emission or manager commands into
the v15 package.

### 6.1 One instruction model, down to execution

The executable handler now carries the same `DispatchStep` leaves as HIR.
The duplicate `Step` enum and statement-copy bridges are removed, without a
compatibility alias. Lowering substitutes constants inside that shared model;
it no longer relies on `unreachable!` to convert its additional variants.
`HandlerExecutor` now names the existing handler transaction preparer, leaving
Executor as the semantic category shared by Operators and Connectors.

Business-output traversal preserves authored target order and does not mistake
manager commands for ports. Expression traversal visits a broadcast builder
once and tuple builders in order; time-demand and metadata-read proofs use the
same allocation-free traversal. Type and annotation propagation inspect each
emission destination separately.

The runtime's scratch emission preparer handles both singular emissions and
the ordered multi-emission algebra. A broadcast evaluates one builder, then
creates each target's envelope; a tuple evaluates its builders left-to-right.
An evaluation or target-pin failure aborts preparation before any transaction
record is returned to the VM. This is not yet a source-to-v16 multi-emission
qualification: source lowering and independent raw-plan validation both
refuse multi-emission and manager commands in the v15 package. The extended
raw-handler ordering tree cannot bypass that gate or acquire an executable
v15 spelling. The reader still rejects its new instruction forms.

The handler container is still the v15 guard plus linear body. Replacing that
container with recursive Dispatch, together with the other complete v16
tables, remains below; shared instruction leaves alone do not close R4.

### 6.2 Recursive decision execution, qualified through the Lab

The core's pure `select_dispatch` now evaluates one path through the shared
tree. Closed selection uses bool/enum/union tags without unwrapping the input;
open predicates retain first-true source order. Pure prefixes are evaluated
in sequence and scoped to the selected path. The result carries the chosen
linear statements and their local bindings, not a committed effect. Depth is
bounded without recursive evaluation; invalid prefixes, ambiguous/missing
closed selections and non-boolean/faulting predicates refuse.

The existing expression `Bindings` seam supplies payload provenance, assets
and logical time. A carried payload remains a witness until a selected
decision or statement demands it. No losing branch is evaluated or decoded.

Lab mocks are the first executing consumer. Their clauses are Driver-script
syntax outside the production package, so this does not publish a partial
v16 shape. The existing first-matching mock filter protocol is preserved;
after a clause wins, its recursive decision selects a whole atomic completion
or written silence. The checker proves exactly one final exit **per possible
reply leaf**, not by counting exits across mutually exclusive siblings. Faults
never fall through to another arm or clause. Mock multi-emission and manager
commands retain explicit source deferrals. This is execution evidence for the
shared decision selector, not closure of application Dispatch transport or R6.

The statement gate also closes a pre-existing check/run split: a linear mock
containing `MultiEmit` passed the old linear-shape check, then faulted during
simulation. A regression produces **no diagnostic** with that old gate and
receives the named `E-CHECK-DEFERRED` with the statement capability check.

## 7. Evidence

Seven core tests cover bool, enum and tagged union; unsupported/open domains;
missing, unknown and duplicated cases; empty arms; case/arm order; absence of
input repair; and the maximum 256-member domain. One test enumerates all 585
zero-to-three-arm tables over eight representative case sets and compares
acceptance with an independent flatten/sort/equality oracle.

The existing 200 compiler tests pass with both consumers wired to the proof.
All three `no_std` targets compile. Cortex-M33 was measured before the first
core edit and after the change: both retain 17,844 decode ticks, 2,713 average /
2,966 maximum relay ticks, **129,972 B** heap peak, 67 records, the unchanged
echo hash, and `SPIKE_OK`. The heap margin remains **1,100 B**, not a new budget.

First checkpoint gates: **1,048 workspace tests passed**, zero failures;
`cargo fmt --all -- --check`, workspace/all-targets/all-features clippy with
warnings denied, `cargo doc --workspace --no-deps`, and `git diff --check` all
pass. Existing echo, JSONL and gauge fixture checks pass without a re-bless.

The second checkpoint adds seventeen core Dispatch tests and the expression
payload-forgery regression. They cover scoped facts/locals, pure prefixes,
constructor members, malformed expressions, depth, annotations and ordered
multi-emission. The JSONL integration and golden tests additionally exercise
heterogeneous mock replies through the real source compiler.

Second checkpoint gates: **1,066 workspace tests passed**, zero failures;
fmt, workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. No package, AST or trace
golden was changed. The before/after Cortex-M33 measurements retain **129,972 B**
heap peak (1,100 B margin), 5,640 B after open, 2,713 average / 2,966 maximum
relay ticks, 67 records and the echo hash; decode moves from 17,844 to 17,840
ticks. Both runs finish with `SPIKE_OK`.

The third checkpoint adds eleven core routing tests and two compiler tests.
Besides malformed partitions/sets and cross-declaration conflicts, the core
battery enumerates **4,352** tables over a bool source, two port destinations,
discard and a manager command. Every table is checked in both declaration
orders against an independent selected-value flatten/sort/uniqueness oracle.
Compiler tests keep exact-case source diagnostics, one physical edge per
destination and identical hashes across additive/grouped route spellings.

Third checkpoint gates: **1,079 workspace tests passed**, zero failures;
fmt, workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. No dependency, package version,
Contract version or golden changes. Cortex-M33 before/after is identical:
17,840 decode ticks, 2,713 average / 2,966 maximum relay ticks, **129,972 B**
heap peak (1,100 B margin), 5,640 B after open, 67 records, unchanged echo hash
and `SPIKE_OK`.

The fourth checkpoint adds eight core Call tests and three compiler tests.
Terminal inventories are compared with independently constructed exit types;
unknown identity/profile combinations refuse. The battery checks all **432**
terminal-case/effect/command combinations across the four admissible profiles,
the conservative visibility rows, repeated/single/absent frames, and exact
case selection. Source probes verify the specific cardinality/effect
diagnostics and that valid local proofs do not bypass packaging or the
existing effect-execution gate. The new diagnostic regression failed before
the implementation with only the generic effect deferral, not an admitted
unsafe execution.

Fourth checkpoint gates: **1,090 workspace tests passed**, zero failures;
fmt, workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. No dependency, package,
Contract major or golden changes; the echo, JSONL and gauge witnesses remain
unchanged. Cortex-M33 before/after retains 17,840 decode ticks, 2,713 average /
2,966 maximum relay ticks, **129,972 B** heap peak (1,100 B margin), 5,640 B after
open, 67 records, the echo hash and `SPIKE_OK`.

The fifth checkpoint exercises all five emission forms in a source-authored
scenario before and after package decode. It checks exact plan/hash/encoded
byte equality, constant substitution, repeated writes to one field in authored
order, metadata patching, fresh struct/scalar outputs, `nothing`, and identical
journal records on both executions. Existing proof, annotation, type and
runtime tests now consume the shared types directly.

Fifth checkpoint gates: **1,091 workspace tests passed**, zero failures;
fmt, workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. No dependency, package,
Contract major, golden or witness identity changes. Cortex-M33 before/after
retains 17,840 decode ticks, 2,713 average / 2,966 maximum relay ticks,
**129,972 B** heap peak (1,100 B margin), 5,640 B after open, 67 records,
the echo hash and `SPIKE_OK`.

The sixth checkpoint adds eight tests: three shared-instruction traversal
tests, four scratch evaluation tests and a forged raw-plan refusal covering
broadcast and all three manager commands. The depth oracle additionally
compares every new instruction/builder combination against the actual raw
canonical tree, then checks that none can pass the v15 executable gate.

Sixth checkpoint gates: **1,099 workspace tests passed**, zero failures;
fmt, workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. No dependency, accepted
package shape, Contract major, golden or witness identity changes. Cortex-M33
retains **129,972 B** heap peak (1,100 B margin), 5,640 B after open, 67 records
and the echo hash. Decode changes from 17,840 to 17,848 ticks; relay changes
from 2,713 average / 2,966 maximum to **2,710 average / 2,962 maximum**. An
intermediate binding lifetime extended through emission staging measured
2,992 maximum ticks; releasing it after evaluation restored the margin without
an extra cache or altered semantics. Linked `.text + .rodata` is 731,672 B,
below 1 MiB. The final measurement prints `SPIKE_OK`.

The decision-execution checkpoint adds twelve tests: seven core selector
tests; a per-leaf mock-completion checker matrix; the unsupported mock
multi-emission regression; and three Lab tests covering nested lazy prefixes,
faults without fall-through, and an end-to-end recursive/linear mock twin.
That twin preserves both the production semantic hash/plan and every causal
record of the three scenarios, including timeout and filtered input.

Decision-execution gates: **1,111 workspace tests passed**, zero failures;
fmt, workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. No dependency, package,
Contract major, golden or existing witness identity changes.

Cortex-M33 retains 17,848 decode ticks, 2,710 average / 2,962 maximum relay
ticks, **129,972 B** heap peak, 5,640 B after open, 67 records, the echo hash,
and `SPIKE_OK`. Linked `.text + .rodata` is 731,656 B. The selector does not
add a walk to an existing unannotated Operator firing; application Dispatch
still requires the complete categorical transport below.

## 8. Remaining R4 work

R4 remains open. The complete representation must still carry recursive
Dispatch in the encoded plan, case route selections separate from physical
edges, canonical dispositions, manager commands, Extension specifications,
Loop r2 identities and causal facts, and the six Contract `@2` movements. Independent validation
must cover each transported claim, including composition of the local
expression/narrowing proof with independently resolved plan context. R5–R8
still own productibility, execution and Loop recovery/qualification.

Only when those shapes are complete does R4 allocate the **single** categorical
v16 move and enumerate witness changes under
[`rework-r0-identity.md`](rework-r0-identity.md). No partial v16 format ships to
make these validation checkpoints look complete.
