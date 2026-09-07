# R7 — manager contexts and spatial recovery

> Work in progress, 2026-09-07. This checkpoint strengthens the r2 journal
> boundary used by the existing Operator/Call Loops. It does **not** open the
> Graph Loop execution gate or qualify complete Worker recovery.

## 1. Invariant and repository contact

A Loop packet is part of one causal transaction, not a freestanding claim
about a run. Its carrying Executor must belong to the exact run instance
scope; manager facts belong to that run's owner; routed contexts refer to
deliveries actually retained by that transaction; exhaustion is published as
the same ordinary `Loop.expired` emission.

Three executable probes contradicted this invariant before the change:

- changing a Loop transaction's owner to the enclosing App still passed the
  independent r2 replay fold;
- adding a context for a nonexistent delivery still passed;
- deleting the public expiry emission while keeping the Expired fact still
  passed.

The protocol-only fold had verified command identities, ordinals and phase
transitions, but did not inspect their carrying record. These are journal
acceptance/replay defects, not a claim that the normal Worker produced such
records. The regressions first failed because replay accepted the forgeries.

## 2. Record-aware fold

`Fold::apply_transaction` is now the acceptance path for MemJournal, independent
replay and the existing persistent-Operator recovery path. It checks the
packet and its record in one scratch fold, publishing neither on refusal.
The lower-level `Fold::apply` still checks packet algebra in isolation;
it is not a journal-acceptance API.

The fold derives active `InstanceRef -> parent InstanceRef` links from the
existing `l:start`/`l:stop` records. It does not infer scope from a lexical path
prefix or a business origin. Two tenants with identical owner paths remain
distinct, and a successor parent generation cannot repair a child's link to
the preceding generation. Context stacks follow these retained links from
outermost to innermost; repeated runs and reversed nesting refuse.

For each r2 record:

1. inherited context runs must be active and enclose the transaction instance;
2. each manager fact must name the carrying owner instance;
3. each routed context must match exactly one retained
   `delivery_ref(edge, envelope_id)` occurrence;
4. an Expired fact must have exactly one matching `Loop.expired` emission,
   with index zero and the exact retained envelope.

Refusals are typed: `ContextOwner`, `ContextDelivery`, `ExpiryEmission`, plus
the existing protocol errors. A failed check leaves records, pending commands
and the cached fold unchanged. Recovery performs the same check in its
candidate VM before replacing the live VM.

The cached projection is created lazily when the first r2 packet is accepted,
seeding its parent links from the committed prefix. Later lifecycle changes
update it atomically. A journal without r2 work retains no new index; an
explicit fold query may derive the lifecycle links on demand. These are
disposable indexes, not another journal or new source of generations.

## 3. Evidence and identity

The three integration regressions check **both** independent replay and actual
MemJournal commit, require exact error variants, and prove atomic refusal.
They cover every manager/application r2 row in a fuel-exhausting run and four
expiry-publication forgeries: missing output, wrong port, wrong index and
duplicate output.

Four core tests exercise valid descendant contexts, generation isolation,
two keyed tenants at the same lexical paths, stack order and repeated runs,
and exact edge/envelope occurrence matching. The existing pending-command
recovery test now includes its actual parent/start facts rather than presenting
isolated manager packets as a complete causal record history. No test was
ignored or dropped. Existing integration tests still restore every committed
prefix and compare all subsequent records without rerunning producers.

No source, package, Contract, record-field or causal recipe changes. The parent
links already exist in lifecycle facts; key 55 already carries the r2 packet.
Echo, JSONL and gauge candidate bytes and all scenario seals are checked against
the preceding checkpoint; no golden is re-blessed.

Executed checks on this checkpoint:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check`, `git diff --check` | pass |
| workspace/all-targets/all-features clippy, warnings denied | pass |
| `cargo doc --workspace --no-deps` | pass |
| `cargo test --workspace --all-features --no-fail-fast` | **1,162 passed, 32 failed**, 31 suite reports, zero ignored |
| core / v16 Loop suites | **310 / 25 passed** |
| no-std: `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` | pass |
| candidate echo/JSONL/gauge ledger | exact package bytes and every causal seal unchanged |
| emulated Cortex-M33 | `SPIKE_OK`; **106,244 B** peak, **24,828 B** below the 128 KiB gate |

The failing test-name multiset is unchanged: 25 old Loop/Graph/UI-form tests
and seven categorical expectations. Seven new regressions pass; no test or
golden was removed to obtain that result.

The embedded witness was rerun before the first core edit and after it, with
the same candidate package and 67 records. Heap peak and 6,440 B after open
are unchanged. Relay timing remains 1,462 / 1,415 / 1,656 ticks
(average / minimum / maximum), calibrated against 39,999 ticks per 2,000,000
instructions. Decode changes from 21,270 to 21,269 ticks. Text 738,276 B plus
read-only data 64,688 B totals **802,964 B**, an increase of **6,056 B**, below
1 MiB. The non-Loop witness allocates no context index; these measurements do
not qualify the future Graph Loop workload.

## 4. Remaining execution gates

This scope check is **not** complete delivery-context validation or quiescence.
The following still require implementation and end-to-end evidence together:

- retain the exact opening Graph-boundary admission before expansion, so its
  internal fan-out opens one run rather than one run per flattened edge;
- propagate and re-derive per-delivery contexts from consumed positions,
  including push/pop at Loop boundaries and rejection of cross-context joins;
- derive `Loop.state` and commands from that exact consumed context, not merely
  from lexical containment;
- count run-scoped pending deliveries, Calls, timers and child-manager work,
  excluding downstream work after a boundary pop and the selected command itself;
- gate command consumption and recurrence on that complete obligation set;
- restore Graph admissions and the remaining temporal/exchange obligations from
  retained facts, then qualify all three host traces from R0.

The source and independent plan gates still refuse Graph Loop orchestration.
Passing low-level nested-context tests does not override those gates. The full
workspace also retains its old-form and categorical-fixture migration failures.
