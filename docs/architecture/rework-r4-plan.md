# R4 — categorical plan and independent validation

> **Status (2026-09-05): open, first validation checkpoint only.** R3 is
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

## 3. Evidence

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

Final checkpoint gates: **1,048 workspace tests passed**, zero failures;
`cargo fmt --all -- --check`, workspace/all-targets/all-features clippy with
warnings denied, `cargo doc --workspace --no-deps`, and `git diff --check` all
pass. Existing echo, JSONL and gauge fixture checks pass without a re-bless.

## 4. Remaining R4 work

R4 remains open. The complete representation must still carry recursive
Dispatch, case route selections separate from physical edges, canonical
dispositions, manager commands, Extension specifications, Loop r2 identities
and causal facts, and the six Contract `@2` movements. Independent validation
must cover each transported claim, including the expression/narrowing proof,
not just case partitions. R5–R8 still own productibility, execution and Loop
recovery/qualification.

Only when those shapes are complete does R4 allocate the **single** categorical
v16 move and enumerate witness changes under
[`rework-r0-identity.md`](rework-r0-identity.md). No partial v16 format ships to
make this first checkpoint look complete.
