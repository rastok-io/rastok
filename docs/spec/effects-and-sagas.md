# Effects and the Saga boundary

> **Status (2026-09-05): effect classification is normative; Saga execution
> is a named future gate.** D-2026-09-05-1 removes Connector handlers and the
> former executable-looking `on compensate` surface. No v16 program may
> claim Saga semantics until the independent Saga decision closes every gate
> in §5.

> **Approved next refactor (2026-09-14):**
> [D-2026-09-14-4](decisions.md#d-2026-09-14-4--separate-loop-retry-and-saga-over-one-recurrence-engine)
> removes the global Connector `effect` requirement at cutover. An ordinary
> Call needs no Saga classification; operation idempotence governs Retry
> safety, while compensation and irreversibility belong to the future Saga
> step contract. The classification and gates below describe the pre-cutover
> implementation, not the approved target. The
> [refactor ledger](../architecture/retry-extension-refactor.md) tracks their
> replacement; Saga execution remains unimplemented.

This document owns the static classification of external effects and the
minimal direction reserved for the future Saga Extension. Loop repetition
safety is specified in [loop.md](loop.md).

## 1. Effects live on Connectors

Operators compute and mutate only journaled semantic state. Graphs describe
scope and topology. Only Connectors communicate with external systems, so the
`effect` axis belongs to the Connector's resolved Contract profile.

```text
Effect = none | idempotent | compensable | irreversible
```

| Effect | Forward repetition | Inverse operation | Meaning |
|---|---|---|---|
| `none` | safe | none | no external side effect |
| `idempotent` | safe under the Contract | unnecessary | repeated operation has the same external meaning |
| `compensable` | not implied safe | required by future Saga contract | a distinct inverse operation can repair the forward effect |
| `irreversible` | unsafe unless the partner deduplicates the same operation identity | impossible | point of no return |

The label describes semantics, not an HTTP verb, transport, or implementation
technique. A Contract may fix it for a profile or require the declaration to
choose it. Missing required classification fails closed.

`behavior = deterministic | probabilistic` remains a separate Connector
attribute. Determinism says whether identical admitted facts determine the
same logical outcome; effect says what repeating the external operation does.

## 2. Journal-before-effect

An external command follows one order:

1. pure preparation resolves every setting, request byte, operation identity,
   target instance, and required capacity;
2. the producing Executor transaction commits;
3. an OUTBOUND open is retained when the exchange requires one;
4. only then may the Driver make bytes or another effect visible.

The Driver never mutates semantic state or the journal. A correlated outcome
returns through the bounded Connector activation path and closes the existing
OUTBOUND obligation; it is not fabricated as a second INBOUND root.

Open effects whose outcome cannot be proved after recovery close loudly as
`OutcomeUnknown`. Delivery semantics, idempotency-key stability, and
close-once rules live in [delivery-semantics.md](delivery-semantics.md).

## 3. Static companion rules

### 3.1 Idempotent

No extra syntax is universally required. The Connector's Contract must state what
makes repetition idempotent. If that depends on an operation key, the key is
part of the prepared request seal and recovery must reuse byte-identical
material.

### 3.2 Irreversible

An irreversible outbound operation requires a stable
`idempotency_key` whenever its profile can be retried or recovered. A key
which changes by Loop iteration does not make an ambiguous prior attempt safe:
it names a new business operation.

### 3.3 Compensable

`compensable` does not authorize forward repetition. In particular,
Loop Next/Again after `outcome_unknown` or `after_effect` is rejected;
only `effect = none | idempotent` is repeat-safe there.

The compensation itself is a different external operation and therefore needs
its own stable identity. Reusing the forward idempotency key may cause a
partner to deduplicate the inverse into the forward operation. The future
Contract compensation block must make that distinction explicit.

There is no Connector DSL handler. These historical forms are rejected with a
migration diagnostic:

```rastok
on compensate { ... }
connector x : SomeContract { on in { ... } }
```

A compensable Connector's Contract will eventually contribute a versioned
`compensate = { ... }` contract understood by its core transition and Driver
binding. Exact syntax and fields are not admitted yet.

## 4. Loop interaction

Every terminal case of a finite outbound Call profile is classified:

```text
before_effect | outcome_unknown | after_effect
```

Recurrence after `before_effect` is allowed for every effect class because
the contract proves nothing became externally visible. Recurrence after
`outcome_unknown` or `after_effect` requires `none` or `idempotent`.
`compensable` and `irreversible` refuse with `E-LOOP-EFFECT`.

This is checked per productive terminal case, including a conservative join
of any frame facts that can select a Loop command. A Driver cannot downgrade
the class at runtime.

## 5. Saga Extension — reserved direction, not v16 execution

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
| `E-EFFECT-MISSING` | a Connector's Contract profile requires an explicit effect and none was supplied |
| `E-IDEMPOTENCY-KEY` | a repeatable/recoverable irreversible operation lacks stable identity |
| `E-LOOP-EFFECT` | recurrence could repeat an unsafe visible effect |
| `E-CHECK-DEFERRED` | Saga or compensation syntax is recognized but not executable |

The precise existing diagnostic registry remains authoritative until R3
allocates or consolidates codes.

## 8. Related documents

- [extensions.md](extensions.md) — Extension ownership and admission.
- [loop.md](loop.md) — finite recurrence and effect-stage checking.
- [delivery-semantics.md](delivery-semantics.md) — OUTBOUND WAL, recovery,
  close-once, and unknown outcomes.
- [stdlib.md](stdlib.md) — Connector-Contract-specific effect requirements.
- [instances.md](instances.md) — lifetime, Closing, and journal-derived
  generations.
