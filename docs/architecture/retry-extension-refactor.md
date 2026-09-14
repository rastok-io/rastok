# Loop, Retry, Call and Saga — refactor contract

14 September 2026. **Semantic direction approved; implementation not started.**
[D-2026-09-14-4](../spec/decisions.md#d-2026-09-14-4--separate-loop-retry-and-saga-over-one-recurrence-engine)
owns the decision. This document records the repository contact, implementation
order and acceptance evidence still required. Current packages remain v18.

> A Loop advances a state; a Retry repeats an operation. The first proves
> finite iteration; the second proves safe repetition.

Retry is also finite: its attempt bound includes the first Call. Neither
mechanism promises that a remote operation has ceased merely because its
local Call timed out. Reentrancy is a different property, concerning a call
made while a previous invocation remains active, not permission to repeat it.

## 1. Responsibilities

| | Loop | Retry |
|---|---|---|
| Repeated subject | a program-selected state | one retained prepared operation |
| Decision | the body's Next/Again/Done verdict | terminal case selected by a static policy |
| Proof | fuel or a supported decreasing measure | terminal visibility and idempotence; finite attempts |
| Exhaustion seen by the parent | the existing host-terminal `LoopExpired` case | the last failure unchanged |
| Authored surface | `Loop.state`, commands and attributes | attributes only; no state binding, command or port |

`App` remains the public root Extension. A Call remains a Connector exchange,
not another Extension. Saga remains the independent future compensation
protocol. Graphs acquire no application handler, mailbox or transaction.

No global `effect` declaration should prevent an ordinary Call from executing
once. `behavior` remains the determinism axis. The operation's `idempotent`
guarantee is separate: absent/false means no guarantee, not proven impurity.
Its source and fixed/author-declared admission follow the exact Contract;
the checker cannot prove a remote partner's truthfulness. A partner key alone
does not supply the guarantee. Saga may use the same guarantee for forward
and compensation Calls without reclassifying idempotence as compensability.

## 2. Retry source and admission

The following is a future-syntax fragment, not a runnable v18 program:

```rastok
connector pay : HttpClient + Retry {
  idempotent = true
  idempotency_key = meta.app.payment_id
  Retry.attempts = 3
  Retry.delay = {
    kind = exponential
    first = duration("100ms")
    ceiling = duration("2s")
  }
  Retry.on = [ConnectFailed, TimeoutExpired]
  // Ordinary HTTP request, codec, behavior and timeout settings follow.
}
```

- `Retry.attempts` is a positive plan constant: 3 means at most three Calls,
  not one initial Call plus three retries.
- `Retry.delay` uses the existing fixed/exponential logical-delay vocabulary,
  with bounded arithmetic, no jitter and no hidden clock.
- `Retry.on` is a canonical static set of the resolved terminal's cases.
  There is no handler, arbitrary expression, payload predicate or alias
  `Retry.cases` in this first profile.
- A selected `before_effect` case may recur without idempotence. A selected
  case with possible/completed external visibility requires the guarantee.
  Compensation does not make an unsafe repetition safe.
- The initial host is a finite outbound Call with bounded completion storage
  and a positive timeout. Event, Session, Process service, inbound Call,
  Operator and whole-Graph retry remain named, unadmitted profiles.

HTTP 500 is `Completed { status: 500 }`, not a transport-failure case.
Selecting `Completed` selects that entire case, not only non-2xx responses.
Payload-selective retry requires a separate decision; the new Extension must
not silently change the flagship's explicit HTTP-status policy.

## 3. One operation, several retained attempts

The manager pins the prepared request and the settings that define the
operation: target, method/path, body, ordinary headers and idempotency material
where applicable. Invocation expressions are not re-evaluated to manufacture
a different operation between attempts. The existing secret exception stays:
logical references and immutable binding attestations are retained, never
clear credentials in the journal. Retry does not silently rotate a binding.

Each attempt receives a new Call identity and exactly one closure. All
attempts are correlated with the exact opening admission and owner generation;
business `origin_id` alone cannot distinguish two deliveries of one root.
Within one Retry, changing that pinned request must refuse before another
effect. This is not a new global deduplication registry. The operation's key
is not incremented with the attempt ordinal.

Only the terminating attempt publishes its ordered host outputs and terminal
to the graph. That may be success, an unselected terminal case, or the last
selected failure after the attempt bound. The last failure's type/payload is
not replaced with `RetryExpired`; ordinary envelope identity and causality
still reflect the retained producer. There is no new port.

Every intermediate attempt remains an OUTBOUND fact with its real completion.
The selection and publication decision must be reconstructible and accepted
exactly once. Recovery between settlement and publication must neither lose a
final result nor publish it twice. A late outcome of an already closed attempt
cannot win over its retained closure or settle another attempt.

## 4. One recurrence manager and fold, distinct policy causes

The refactor shares the manager and journal-derived recurrence foundation,
not merely the delay helper. Common obligations are opening, exact admission,
bounded ordinal progression, waiting for a retained due time, quiescence,
capacity admission, generation fencing and closure. There is no separate
Retry ledger or mutable counter authority.

The policy-specific payload remains truthful:

- Loop supplies a state envelope and an authored command reference.
- Retry supplies the pinned operation and the exact closed Call whose
  terminal selected the static policy.

Thus Retry has internal protocol state, but no program-owned state or
`Retry.state`. It cannot produce fictitious `CommandProduced(Again)` facts.
The common fold must represent these two causes explicitly and reject a
policy/cause mismatch. Loop's command production/consumption crash window
remains covered; Retry's settlement/next-admission window is covered without
pretending that an Operator made the decision.

Current `journal/loop_protocol.rs` is not yet that common algebra: `Advance`
requires a `CommandRef` and an envelope state, and the fold validates them.
Historical r2 packets cannot simply be relabelled Retry packets. The versioned
refactor must enumerate the common facts and policy-specific payloads before
allocating or blessing their encoding. Shared facts do not require every
policy to emit every kind of fact: Retry never emits a Loop exhaustion value.

Closing wins over a due retry; nothing migrates into the owner's successor
generation. Quiescence is established from retained obligations, not inferred
from wall-clock silence. Replay/recovery reconstructs the manager without
calling an old Driver effect; idempotence alone is not an execution command.

## 5. Composition

Containment is `Loop -> Retry -> Call`, never `Retry -> Loop`. An ordinary
Loop Graph containing a Retry Connector is the initial composition target.
A same-owner `Loop + Retry` needs the same unambiguous nesting and qualification,
independent of Extension spelling order; it is not admitted by manager ordering.
Retry attempts do not consume extra Loop fuel: they belong to one application.

In a future Saga, multiple attempts of one operation are not multiple successful
steps. Separate Loop applications can undertake separate operations and thus
incur separate compensation obligations. A compensation is a new operation,
with an identity distinct from the forward operation and its own repetition
guarantee. Neither an unknown outcome nor HTTP `Completed` by itself establishes
business success. Saga composition, step declarations, pivots and compensation
execution remain the separate Saga gate, not work smuggled into Retry.

## 6. Accounting and inspection

Graph publication and accounting visibility are different decisions. Every
attempt's observed usage remains attributable to that attempt and operation,
including failed attempts withheld from downstream ports. The final attempt
must not be counted once from its closure and again from its published output.
Absent usage/prices remain unknown/unpriced; Retry never invents zero cost.

The existing CostReport path must be audited before output filtering lands:
retained raw completions alone are not proof that the current report extracts
their annotated measures. Policy validation and redaction apply equally to
intermediate completions. No new counter, reconstructed fake envelope or
observation stream becomes the accounting authority.

The debugger must distinguish an iteration, an operation and an attempt, and
show all retained attempts without drawing Retry commands or ports. Physical
time and repeated billing are not made deterministic by idempotence. The
ordinary one-model HTTP adapter needs neither Saga nor a false idempotence
claim to make a single invocation after the global effect gate is removed.

## 7. Contact with the repository

| Current authority | Consequence for the refactor |
|---|---|
| [`node::Extension`](../../crates/rastok-core/src/node.rs) and [`plan::ExtensionSpec`](../../crates/rastok-core/src/plan.rs) are closed | add Retry only with complete source, transport and independent validation; no speculative tag |
| [`TerminalVisibility`](../../crates/rastok-core/src/connector/call.rs) already classifies each terminal | retain the Contract facts; separate their consumer from the global effect enum |
| [Compiler check](../../crates/rastok/src/compiler/check.rs) admits only `none`/`idempotent` effect words | remove the global effect gate at cutover, not by calling paid inference idempotent |
| [Plan validation](../../crates/rastok-core/src/plan/validation.rs) and [VM Call dispatch](../../crates/rastok-core/src/runtime/vm.rs) require an effect class for outbound callees | removing an attribute is insufficient; rebase dispatch on Contract roles/exchange |
| [`Intent`](../../crates/rastok-core/src/boundary.rs) retains `EffectClass`; [Call recovery](../../crates/rastok-core/src/runtime/vm/recovery/calls.rs) checks equality | audit plan, causal seal and recording together; do not reinterpret a class tag as a boolean |
| [Loop protocol](../../crates/rastok-core/src/journal/loop_protocol.rs) has Loop state and command-shaped causes | generalize the recurrence algebra; never synthesize a command for Retry |
| [Stdlib Process job](../spec/stdlib.md) publishes a bounded completion at exit | use the existing completion boundary for selection; do not claim unqualified streaming retry |
| [Extension admission](../spec/extensions.md#5-extension-admission-test) requires dedicated orchestration and closed composition | pinned attempts and selective final publication justify Retry as a protocol, not Loop sugar |

The normative clauses to supersede are the global effect requirement and
Loop effect-safety coupling in D-2026-09-05-1, effects-and-sagas, loop §11,
the outbound recovery descriptions, and affected Contract sheets/identity
tables. Node ontology, ordinary routing, user data annotations, Call close-once,
logical time, no-recall replay and Saga's execution deferral stay intact.

## 8. Implementation order and exits

| Slice | Required result | Status |
|---|---|---|
| C0 — contract and contact | approved semantic distinction, output publication, audit and migration inventory | recorded here; exact encoding still unallocated |
| C1 — operation and recurrence algebra | sealed-request binding, common fold with typed decision causes, identity register and crash traces | not started |
| C2 — source and validated plan | Retry settings, idempotence, removal of global effect, full registry-free checks, one declared format cutover | not started |
| C3 — runtime and accounting | qualified Call attempts, final publication, shared scheduling/recovery, exact per-attempt report input | not started |
| C4 — migration and qualification | examples, goldens with reasons, UI, docs, no_std and memory/code-size gates | not started |

C1 must enumerate package/Contract/recording movements and non-movements before
any re-blessing. No old tag receives a new meaning. No claim is made that all
Call-bearing witnesses remain byte-identical when their effect representation
changes; expected differences must be recorded first. Unaffected pure witnesses
retain their identity. No new format number or hypothetical public trait is
allocated by this document.

The acceptance matrix must cover:

1. attempts 1 and 3, unknown/duplicate terminal selections and invalid bounds;
2. before-effect repetition without idempotence; ambiguous repetition refused
   without the guarantee; a key alone not waiving it;
3. frozen requests/settings across attempts and distinct Call identities;
4. intermediate failure/data withheld from the graph but retained/accounted;
   last failure unchanged; a final result published exactly once;
5. crash prefixes after settlement, before/after scheduling, before final
   publication, plus late outcomes and Closing at the due instant;
6. two keys and two same-origin admissions with independent Retry runs;
7. a Loop application containing several Retry attempts, with fuel charged
   once for that application; no reverse containment;
8. missing usage, exact decimal costs, secret/personal redaction, no double
   counting from closure plus final publication;
9. CLI, Lab batch/step, diagnostic replay and debugger agreement;
10. workspace checks, three no_std targets, embedded heap/code size measured
    before growth is accepted, and explicitly explained golden migrations.

No implementation, runtime qualification, benchmark call, paid model invocation,
release tag or public push is claimed by this preparatory slice.
