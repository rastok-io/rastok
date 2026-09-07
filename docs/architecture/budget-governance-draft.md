# Draft — budget governance after M1

> **Non-normative design memory, 2026-08-30.** This file preserves decisions
> explored while scoping M1.8. It reserves no source spelling, Extension,
> package tag, journal key, exit case or release milestone. M1.8 delivers the
> annotation, secret, measurement and cost foundations only.

> **Historical pre-v16 syntax.** The Graph handlers in the sketches below
> are design history, not executable examples. v16 Graphs have no handlers;
> a future Budget design must express these decisions through case routing
> and child Operators under the current [Extension model](../spec/extensions.md).

## Why this is separate

Metering answers what a flow consumed and cost. Governance must decide before
an effect whether the system may spend more. OpenTelemetry arrives after or
during work and therefore cannot authorize it. A future budget protocol must
reserve against a durable authority before starting a Driver, then settle from
the observed outcome without treating missing usage as zero.

Budget limits also do not belong in graph source. Recharging a tenant or
changing an operational quota must not rebuild, re-hash and redeploy the
graph. Source may eventually declare that an owner participates in the
protocol; deployment/control-plane state supplies the current policy.

## Candidate shape

The leading design is a `Budget` Extension because governance changes
execution, port typing and journal obligations rather than merely configuring
a value:

```rastok
graph agent : App + Loop + Budget {
  instance = { key = meta.app.session_id }

  connector model : HttpClient {
    // No monetary limit is embedded here.
  }

  on model.exit where data is BudgetRefused {
    cheaper_model.in = Loop.state.data
  }
}
```

The exact syntax is deliberately unreserved. A deployment profile could bind
the Extension owner to a live policy and account:

```json
{
  "budgets": {
    "agent": {
      "policy_ref": "tenant-conversation-v3",
      "account_ref": "billing/tenant-42",
      "limits": {
        "calls": 100,
        "money": { "amount": "1.000000000000000000", "currency": "EUR" }
      }
    }
  }
}
```

This JSON is illustrative, not a frozen deployment schema.

## Candidate invariants

1. `+ Budget` is plan-shaping; limit values are deployment state. The Extension
   may therefore affect semantic identity while a recharge does not.
2. The Extension owner must contain at least one governed Call. An inert Extension
   refuses rather than pretending to enforce anything.
3. `BudgetAccountRef` is logical and placement-neutral. One initial option is
   the Extension owner's `InstanceRef`, yielding one account per keyed instance;
   external account references may be needed for cross-run top-ups.
4. One Call reserves against one account in the first gate. Nested overlapping
   budgets refuse until multi-account atomic reservation has a real design.
5. Reservation is written before an OUTBOUND effect. Insufficient balance
   opens and closes a journaled refusal back-to-back, starts no Driver and
   emits one typed empty `BudgetRefused {}` case on the affected Call's `exit`.
6. `BudgetRefused` exists only on Calls statically governed by the Extension, so
   attaching policy changes their resolved exit unions. Ungoverned graphs and
   connector contracts retain their current types and hashes.
7. A refusal is a normal policy verdict, neither an implicit `err` output nor
   a machine fault. The graph may route it to a cheaper model or terminate.
8. Reservation and settlement are fields on the existing journal obligation,
   not a second financial ledger. A fold reconstructs available, reserved,
   settled and refunded quantities.
9. The first safe settlement rule may charge the full reserved ceiling when
   actual usage is unknown. Refund requires evidence; silence never means
   zero.
10. Money is exact decimal plus currency. No float or implicit FX is admitted.
    Any later conversion must name amount, source/target currencies, rate,
    authority and effective timestamp.
11. A `PriceBook`/`CostProfile` is versioned deployment input. Every quote or
    reservation records its profile reference and digest; price changes do not
    rewrite history.
12. Strict policy uses provider-declared or otherwise guaranteed ceilings.
    Calibrated policy may later reserve a conservative quantile learned from
    samples, but estimation is never presented as a guarantee.
13. The journal remains the accounting authority. OTel, OpenCost and FOCUS are
    projections or reconciliation inputs, never the admission lock.
14. An embedded image may omit the Budget capability. A plan carrying the
    Extension then refuses before boot with `UnsupportedExtension(Budget)` rather
    than running without governance.

## Candidate protocol

```text
prepare Call
    │
    ├── resolve policy/account/profile
    ├── compute conservative quote
    └── reserve atomically
           │
           ├── refused
           │     journal open + terminal BudgetRefused
           │     no Driver work
           │
           └── accepted
                 journal OUTBOUND open + reservation
                 start Driver
                    │
                    └── terminal usage/cost evidence
                           settle/refund in closure
```

The typed refusal makes model competition ordinary graph logic:

```rastok
on primary.exit where data is BudgetRefused {
  secondary.in = request
}
```

A Loop retry sees the same ordinary exit. No special Loop/Budget composition
rule is needed beyond proving bounded recurrence and one reservation per Call.

## Foundations supplied by M1.8

- stable measure identities and exact quantities;
- `@measure` projection from provider payload fields;
- `@cost` and exact decimal/currency observations;
- deployment profiles and attested boot bindings;
- replayable `CostReport` by origin and logical instance;
- structured logs and optional OTel projections;
- secret-safe price/provider credentials.

## Questions intentionally left open

- whether account identity is always an `InstanceRef` or a separately bound
  logical reference;
- recharge, periodic reset, rollover and hierarchy semantics;
- one account carrying several currencies versus one currency per account;
- distributed reservation leases and Worker-local quota slices;
- settlement from provider-reported cost versus pricebook calculation;
- partial streaming/Session reservations and per-message settlement;
- Event pricing;
- run-local versus instance-lifetime Loop budgets;
- FX authority and reconciliation timestamps;
- FOCUS export columns and OpenCost allocation contracts;
- calibrated quantiles, drift detection and automatic model selection.

These questions are why Budget is a later Extension rather than an unfinished M1
feature hidden behind defaults.
