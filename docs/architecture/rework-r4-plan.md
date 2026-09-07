# R4 — categorical plan and independent validation

> **Status (2026-09-06): v16 cutover in progress, not qualified.** The working
> tree encodes and independently reads categorical **v16**, with recursive
> Dispatch, route selections, Extension specifications and six `@2` Contracts.
> Sections 2–8 record the earlier field-only checkpoints; §10 states the current
> executable boundary. No package or trace golden has been re-blessed.
> R4–R9 are not closed and the complete workspace test suite is not green.

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

### 8.1 Decision and routing fields

`plan::fields` now encodes and reads the complete shared `Dispatch` and
`RouteRule` fields, including pure prefixes, recursive closed/open arms,
annotations, ordered broadcast/tuple emissions and all three manager commands.
It shares the existing expression, type, value, annotation and envelope-builder
codec; none has a second registry. Command-kind tags have one authority shared
by statement/wiring transport and both HIR identity consumers. These are field
bytes, **not an executable package**, and do not bypass source lowering or
`ValidatedExecutionPlan`.

Field shapes (CBOR arrays; symbolic names below are not serialized):

```text
dispatch  = [1, statements]
          | [2, prefix, scrutinee, [[case_names, dispatch], ...]]
          | [3, prefix, [[predicate, dispatch], ...], fallback]
statement = existing Let=1 / Emit=2 / SetVar=3 / Stop=4
          | [5, ports_in_authored_order, [1, broadcast_builder]]
          | [5, ports_in_authored_order, [2, tuple_builders]]
          | [6, [1, next_builder] | [2] | [3]]
route     = [endpoint, [1, destinations] | [2, [[case_names, destinations], ...]]]
endpoint  = [owner, port_name]
owner     = [1, node_path] | [2, graph_path]
destination = [1, endpoint] | [2] | [3, owner, command_kind]
command_kind = 1 (Next) | 2 (Again) | 3 (Done)
```

Routing rows sort by their canonical encoded bytes, strictly; destination sets
and closed case sets retain their checked canonical algebra order. Readers
reject wrong order rather than sorting a forged claim. Open arms, statements,
builders and multi-emission targets retain authored order. Node paths cannot
be empty; the empty Graph path denotes the root. A manager command cannot be
decoded as a spatial endpoint.

The reader verifies exact tags/arities, consumes the complete byte input,
re-derives the existing local expression/partition/routing proofs against the
supplied independent context, and requires exact canonical re-encoding. It
checks decision depth before constructing children. Encoding additionally
checks the **combined** CBOR depth of decisions, expressions, types, values
and annotations, not merely each component's individual limit.

These field APIs deliberately do not claim owner resolution, whole-plan type
inference, data-policy propagation, output/state-target validity, Loop control,
effect safety or execution capability. Those proofs still belong to the
enclosing v16 plan. In particular, reading command statements does not make
them executable in a v15 handler, and the v15 instruction reader is unchanged.
The package depth proof must include its own enclosing-table framing in
addition to the field's nesting.

Source-to-field tests use the real parser/checker: nested matches and broadcasts
select the same branch after decoding; `where`/explicit-match and closed-arm
permutation twins preserve field bytes and HIR hash; case-routing preserves a
single physical edge shared by several cases. Each new application form still
receives the named categorical-v16 packaging refusal. No witness is re-blessed.

Evidence: sixteen core tests and three source-to-field integration tests.
The full all-feature workspace passes **1,130 tests**, zero failures; fmt,
workspace/all-targets/all-features clippy with warnings denied, cargo doc,
diff checks and all three `no_std` targets pass. The package decoder also
rejects field bytes as `BadMagic`: the two boundaries cannot be confused.

Cortex-M33 before/after retains **129,972 B** peak heap (1,100 B margin),
5,640 B after open, 2,710 average / 2,962 maximum relay ticks, 67 records and
the unchanged echo hash. Decode moves from 17,848 to 17,850 ticks;
linked `.text + .rodata` is 731,712 B. Both measurements print `SPIKE_OK`.

### 8.2 Package integration still open

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

## 9. Candidate v16 allocation (2026-09-06)

The working tree has one categorical reader/writer, version 16; version 15
refuses. The candidate body carries mandatory key 10 for sparse route rules.
Ordinary unconditional edges remain their sole physical authority. Handler
Dispatch tags are Linear=1, ClosedMatch=2, OpenMatch=3; statement tags 5 and 6
carry ordered multi-emission and manager commands. Extension tags are App=2
and Loop-r2=3: historical Loop tag 1, Graph handler owner tag 2 and ChildPort
activation tag 4 are retired, not reinterpreted. Port direction values are
Input=1 and Output=2; old unwired-policy modes are rejected.

Loop specifications transport resolved entry and terminal ports at keys 4/5,
beside fuel/delay/proof. The six standard Contract ids move to `@2`. No new
logical DataType, dependency, role or exchange tag is introduced.

Body key 11 retains canonical unflattened `RouteRule` rows for plans with
nested Graph boundaries. The reader re-composes them and compares the result
to the physical edges and sparse case/manager selections; they are not an
unchecked display-only table. Erasing a nested output disposition together
with its flattened edge still refuses, as does violating a crossed type pin.
Boundary cycles, including unreachable ones, refuse. Composition uses explicit
work stacks so package-controlled boundary depth does not consume call-stack
frames. Root-only non-Graph-Loop plans omit this field; the three candidate
witnesses above are unchanged by its addition.

Ordinary transactions omit the new journal key 55. Its sparse map carries
context=1, routed contexts=2, produced commands=3, consumed commands=4 and
manager facts=5. Open/Advance/Admit/Close/Expired have tags 1–5. Admit retains
the complete admitted envelope; Advance's due instant is retained but remains
outside the causal seal. Done retains Natural/Explicit provenance (tags 1/2);
this is not another DSL command. Domain bytes are run-r2 `0x14`, command
`0x15` and recurrence admission `0x06`; historical domains are untouched.

`package_ledger` compiles and independently decodes each candidate before
reporting it. Current measurements, **not blessed fixtures**:

| Witness | v15 body → v16 body | Candidate semantic hash | Candidate checksum |
|---|---:|---|---|
| echo | 907 → 939 B | `5ffba3ea18c52a98a538a63ed0a7bf23cd3b4be0fa1b7d7acc51ba85dd73cfa3` | `f20579985db83680dfb5f27fe3d0e166600a43e9bf389cdf2e350886e2d68071` |
| JSONL | 951 → 983 B | `2f89c8b5fa22f7fbe489d6aa770d517c3c9db8f92e850028c1e2bb1280efb2fc` | `98ea0ce81a78671ff9c39e474018f5a92f2c923aa5bd9ad350724158b444e72f` |
| gauge | 243 → 249 B | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `36007746ece8b12cef5d4a9bd4f5aefb7b58acbf81f39158c675be7d6d02c2cd` |

Echo/JSONL move because their exact Process Contract and explicit dispositions
move. Gauge's semantic hash and four-record trace seal
`6f463270fbc011a8f355c93505ea426c5697149ca69c692033fc2f88d2d9346c`
remain unchanged. The R0 table's original “body” heading included package
headers; that measurement error is corrected in its ledger.

## 10. Executable boundary and outstanding work

The source → validated plan → encode → decode → VM tests now execute closed
and ordered matches, typed case routing with additive fan-out, ordered
multi-emission and atomic refusal when a later value faults. Losing branches
neither materialize opaque DATA nor acquire an observable `now()` fact.

A fuel-bounded Operator Loop now uses journaled Open/Admit, separately produced
and consumed Next/Again/Done, fresh causal re-admission, typed exhaustion,
logical fixed delay, shared instance state and independent runs. Natural Done
is produced with the terminal emission and consumed by the manager. Closing
records retire pending runs before a stale wake can affect a successor.
The ordinary and join-aware mailbox dispatchers both use this admission gate.

Outbound Call Loops now execute on the Process job, File and HttpClient
contracts. Their exact terminal chooses a routed command or natural Done;
frames do not vote. Retrying a queued timeout retains its opening admission,
and each recurrence arms its own Call timeout. The independent Contract table
checks frame cardinality and terminal effect class. A Driver contradicting
promised cardinality faults with `ContractViolation` even without Loop.

The manager uses the Worker's existing 1,024-slot bound. Active and delayed
runs, plus admitted entries not yet opened, retain their slot until the run
ends. A regression fills all slots with delayed runs, proves that another
ingress refuses without appending a record, then releases the slots and admits
again. The journal's disposable r2 index avoids repeatedly folding the whole
history; every prefix is checked against independent replay, and a later
commit refusal cannot publish a prepared index.

The independent fold refuses duplicate/misidentified commands, repeated
consumption, wrong ordinals/fuel, altered Again state, reused re-admission
identity and fabricated expiry payloads.

The direct integer measure now operates on command Dispatch: facts from open
arms and their false fallthroughs, or a closed bool match, must establish
positivity and a literal strict decrement. `Again` never counts as a decrease.
The compiler derives the sorted certificate and the package validator derives
it independently. Measure-only runs derive `max(n, 0) + 1`; an explicit fuel
still caps execution even when a measure is proved. Forged witness lists and
erased terminal selections refuse.

`Vm::recover_from_journal` now reconstructs persistent Operator instances,
mailboxes, exact consumed cursors, private state, pending commands and Loop
delays. It reads one committed prefix without invoking a handler or appending
anything. A test interrupts after every commit and requires the same restored
inspection and complete subsequent record sequence. Corrupt consumption leaves
the original VM untouched; delayed recovery uses the retained due instant,
not a new interval from restart. Connector obligations, instance closure,
non-persistent lifetimes and timed joins refuse explicitly at this recovery
gate. **This is not complete Worker recovery.** The deadline-retention gap is
recorded in the repository audit.

The first explicit-destination gate is active at source and in core:
possible Operator/Contract outputs must have written dispositions; unproduced
outputs and entirely unreachable Graphs are exempt. An explicit fuel cap can
still exhaust even beside a proved measure. Nested boundary dispositions and
type pins survive flattening and receive hostile-package tests. Finer
case-productibility deductions remain conservative rather than trusting a
serialized impossibility claim.

The Graph application path has been removed from HIR and the VM:
`__graph_handler`, `fire_graph`, `LoopWork`, and the child-observation activation
queue are gone. Retired raw handler-owner/activation variants and their unused
validator/projection branches still need removal; no supported package can
admit them. Graph lifecycle facts remain controller-owned, as required by R0.

Still open, without a claim of closure:

- whole-Graph Loop context/quiescence/command execution and routed commands
  from descendant Executors;
- complete spatial RecoveryFold, including retained temporal obligations;
- finer closed-case productibility proofs and full boundary-context recovery;
- removal of retired raw Graph-handler variants and their dead branches;
- migration of the old Loop/UI fixtures, then the single golden blessing;
- the complete post-cutover qualification matrix.

Latest Cortex-M33 candidate measurement: 130,772 B heap against 131,072 B,
only 300 B margin; worst relay 1,714 ticks at 39,999 calibration ticks per
2,000,000 instructions, approximately 85.7 k instructions against 150 k.
The heap-first gate remains mandatory for subsequent embedded-path changes.

The all-feature workspace suite currently fails on old Loop/UI forms and
unmigrated categorical expectations. Focused passing tests are not presented
as a green workspace or a completed R4–R9.

## 11. Verification checkpoint — 2026-09-06

| Check actually executed | Result |
|---|---|
| `cargo fmt --all -- --check`, `git diff --check` | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo doc --workspace --no-deps` | pass |
| `cargo test --workspace --all-features --no-fail-fast` | **1,145 passed, 33 failed**, 31 suite reports |
| core-only tests | 300 passed |
| v16 Loop / destination / plan-field suites | 21 / 8 / 11 passed |
| no-std checks: `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` | pass |
| emulated Cortex-M33 candidate | `SPIKE_OK`; heap and relay within the bounds above |

The 33 workspace failures are 26 tests still exercising the removed Graph
handler/old Loop and UI-tour forms, plus seven categorical hash/package
expectations. Those tests have not been ignored or weakened to claim closure.
The CLI network-test mutex contains no shared data and now recovers poison,
so one failed tour no longer turns unrelated probes into order-dependent
`PoisonError` failures.

`cargo run -p rastok -- simulate examples/countdown.rkg` executes two passing
scenarios through the public CLI. Its Operator Loop terminates by a re-derived
measure; `Loop.expired` is reported as impossible and needs no artificial
discard. A separate regression encodes and decodes that example before running
the same Lab scenarios. No package or causal golden has been blessed.
