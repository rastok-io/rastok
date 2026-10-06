# Effects and the Saga boundary

> **Format 20 target — D-2026-09-14-4 and D-2026-09-15-1.** Ordinary
> Connectors have no global `effect` classification. Retry uses a plan-owned
> idempotence guarantee; compensation and irreversibility belong to the future
> Saga step contract. `Intent.effect` is removed, with no duplicate
> `Intent.idempotent`. Saga remains a named, non-executable Extension.
> The [common qualification](../architecture/retry-c4-qualification.md) records
> the cutover; no Saga tag or execution is introduced by that work.

This document owns the distinction between operation repetition and future
compensation. Loop's finitude contract lives in [loop.md](loop.md); the exact
Retry protocol lives in the [approved contract](../architecture/retry-extension-refactor.md).

## 1. Effects occur at Connectors; guarantees live in the plan

Operators compute and mutate only journaled semantic state. Graphs describe
scope and topology. Only Connectors communicate with external systems, but
performing an ordinary Call does not require a Saga classification.

`behavior = deterministic | probabilistic` remains the determinism axis.
`idempotent = true` separately declares that repeating the same prepared
operation has the same external meaning under its Contract. Absent or false
means no guarantee, not proven impurity. The checker cannot verify a remote
partner's truthfulness; an idempotency key alone does not supply the guarantee.

The exact Contract decides whether idempotence is author-declared, fixed or
not applicable. Session direction comes from Contract roles/opening facts,
never an `effect = none` sentinel. The physical Driver cannot change either
the guarantee or that opening role at runtime.

## 2. Journal-before-effect

An external command follows one order:

1. pure preparation resolves every setting, request byte, operation identity,
   target instance, and required capacity;
2. the producing Executor transaction commits;
3. an EGRESS open is retained when the exchange requires one;
4. only then may the Driver make bytes or another effect visible.

The Driver never mutates semantic state or the journal. A correlated outcome
returns through the bounded Connector activation path and closes the existing
EGRESS obligation; it is not fabricated as a second INGRESS root.

Open effects whose outcome cannot be proved after recovery close loudly as
`OutcomeUnknown`. Delivery semantics, idempotency-key stability, and
close-once rules live in [delivery-semantics.md](delivery-semantics.md).

## 3. Repetition and compensation are different promises

For a repeated Call, the retained operation pins request bytes, evaluated
settings and any partner idempotency key. Retry does not recompute them at
each attempt. Recovery validates them against the original plan and operation,
not today's source or a guarantee copied into an Intent.

Compensability does not imply safe forward repetition. An inverse operation
is a distinct Call with its own identity and, if retried, its own guarantee.
Reusing the forward idempotency key may cause a partner to deduplicate an
inverse into the forward operation. These are constraints for the future Saga
decision, not a currently executable compensation block.

There is no Connector DSL handler. `on compensate` is removed, not an alias;
a future compensation contract must preserve the no-Connector-code rule.

## 4. Loop and Retry interaction

Every terminal case of a finite outbound Call retains its Contract visibility:

```text
before_effect | outcome_unknown | after_effect
```

A Retry policy may select BeforeEffect cases without idempotence. Selected
OutcomeUnknown/AfterEffect cases require `idempotent = true`, otherwise
`E-RETRY-EFFECT`. All attempts stay journaled; only the final attempt is
delivered. Exhaustion publishes the last failure unchanged.

Loop instead executes authored state progression and proves finitude.
`Loop.again` that may repeat a visible Call without the guarantee produces
the advisory `W-LOOP-REPEAT`, including successful sibling Calls. A wholly
BeforeEffect path does not warn; `Loop.next` is not covered by this warning.
This does not grant safe repetition or weaken Retry's hard gate.

A Loop application may contain Retry operations. Its obligations stay open
through their backoff and final publication; Loop fuel counts the application,
not every internal Call attempt. Reverse composition and whole-Graph Retry
are not admitted profiles.

## 5. Saga Extension — reserved direction, outside M1

Saga is expected to be a temporal Extension on a Graph. The retained direction
is:

- `Saga.compensate` is a qualified Graph output whose closed union has one
  case per compensable step;
- each case carries the journaled forward request and response needed by the
  inverse contract, subject to secret and personal-data policy;
- a compensable Connector's Contract contributes a typed compensation input/contract;
- the Saga Graph routes every case explicitly with a closed route match;
- `-> nothing` is invalid for a required compensable step;
- Saga progress and recovery are folds of ordinary journal facts, never a
  second mutable ledger.

No one of those bullets is executable alone. A future Saga decision must close
all of these gates together:

1. exact definition of a committed compensable step;
2. reverse ordering under fan-out, joins, and nested Graphs;
3. command-to-result correlation and acknowledgement;
4. one terminal typed outcome and close-once arbitration;
5. failure and bounded retry of the inverse operation;
6. recovery after command production, effect visibility, and acknowledgement;
7. pivot/post-pivot validation;
8. interaction with instance Closing and durability;
9. nesting semantics;
10. treatment of `@secret` and `@personal` fields in compensation evidence;
11. Extension composition, especially `Saga + Loop`;
12. exact plan and causal identities.

Until then:

- `: Saga` is a recognized but non-executable Extension;
- `Saga.timeout`, `Saga.rollback`, `Saga.exit`, and previous lifecycle
  cases are not accepted contracts;
- `on compensate` is removed, not retained as an alias;
- no conformance test or documentation may claim rollback recovery.

## 6. Future shape

The following is explanatory direction only:

```rastok
graph payment : Saga {
  connector reserve : Inventory {
    effect = compensable
    compensate = { operation = release }
  }

  Saga.compensate -> match {
    Reserve -> reserve.compensate
    _       -> ...
  }
}
```

The final design may refine names and payloads. It must preserve explicit
routing, distinct forward/inverse operation identities, journal authority,
and the no-Connector-code rule.

## 7. Diagnostics

| Diagnostic | Meaning |
|---|---|
| `E-CONTRACT-ATTR` | the removed global `effect` attribute is not an alias for idempotence |
| `E-RETRY-EFFECT` | a Retry policy selects an ambiguous/visible terminal without the guarantee |
| `W-LOOP-REPEAT` | authored Again may repeat a visible Call without the guarantee |
| `E-CHECK-DEFERRED` | Saga or compensation syntax is recognized but not executable |

[diagnostics.md](diagnostics.md) owns the diagnostic registry. The former
`E-EFFECT-MISSING` and `E-LOOP-EFFECT` are not current admission rules.

## 8. Related documents

- [extensions.md](extensions.md) — Extension ownership and admission.
- [loop.md](loop.md) — finite recurrence and advisory repetition warnings.
- [delivery-semantics.md](delivery-semantics.md) — EGRESS WAL, recovery,
  close-once, and unknown outcomes.
- [stdlib.md](stdlib.md) — exact Contracts, roles and idempotence profiles.
- [instances.md](instances.md) — lifetime, Closing, and journal-derived
  generations.
