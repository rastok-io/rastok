# R4 — categorical plan and independent validation

> **Status (2026-09-05): open, second validation checkpoint.** R3 is
> committed separately from the paused M1.9 UI. The encoded package remains
> **v15**; no v16 tag, journal key, Contract major or golden is allocated by
> this checkpoint.

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

## 4. Evidence

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

## 5. Remaining R4 work

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
