# Draft — budget governance after M1

> **Direction approved, outside M1 — D-2026-09-18-2.** Governance is the
> future `Budget` Extension. The App Graph is the preferred attachment point
> to explore, not an exclusive host rule or a decision about account scope.
> The reservation/settlement protocol below remains a proposal; no Budget
> implementation or qualification is required to close M1.

> **Later scope amendment — D-2026-09-16-7:** CostReport, automatic pricing
> and subset arithmetic are removed from M1. References below preserve the
> earlier design/qualification history; [observability.md](../spec/observability.md)
> and [ui.md](../spec/ui.md) own the current measurement-retention contract.

> **Non-normative design memory, 2026-08-30.** This file preserves decisions
> explored while scoping M1.8. It reserves no source spelling, Extension,
> package tag, journal key, exit case or release milestone. M1.8 delivers the
> annotation, secret, measurement and cost foundations only.

> **Historical pre-v16 syntax.** The Graph handlers in the sketches below
> are design history, not executable examples. v16 Graphs have no handlers;
> a future Budget design must express these decisions through case routing
> and child Operators under the current [Extension model](../spec/extensions.md).

## Current proposal — 18 September 2026

**Proposal for review, not an approved execution protocol.**
[D-2026-09-18-1](../spec/decisions.md#d-2026-09-18-1--separate-generic-measurements-otlp-export-and-budget-authority)
settles a generic `@measure(id, unit)` target and a future optional `otlp`
observer. Neither enforces spending. The sections after this update preserve
the August design exploration; their source spelling, refusal shape and
unknown-outcome settlement are not commitments.

[D-2026-09-18-2](../spec/decisions.md#d-2026-09-18-2--reserve-budget-governance-for-a-post-m1-extension)
subsequently settles the Extension name/category and its exclusion from M1.
It does not approve the remaining protocol choices.

### Three separate capabilities

| Capability | Input and authority | Delivery status |
|---|---|---|
| Retain measurements | Typed numerical fields, validated annotations and original causal journal records | M1 foundation; the new two-field signature still needs its explicit migration |
| Show consumption and cost | Selected original samples, causal attribution, reported amount or explicit versioned valuation policy | Later read-only consumer; no M1 report or automatic tariff application |
| Enforce a budget | Account policy and an atomic reservation before the effect, followed by settlement | Later semantic protocol; not OTLP and not implemented |

A cost is not a new numerical type. Under the target signature, an amount may
be `decimal(18,6) @measure(id = amount, unit = "USD")`, just as tokens use
`unit = "token"`. An id and a unit do not say whether that value is a supplier
charge, an estimate, a balance or a reservation. A financial consumer must bind
its evidence explicitly to the producing Contract/port/path and causal scope.
It must not sum every matching field in the graph.

There are two valuation paths: inspect a provider-reported amount if it exists,
or calculate from measured quantities under a retained, versioned price
profile. For example, input and output tokens can have different prices.
No price is inferred from their names. A calculated estimate and a reported
charge for the same attempt are alternatives for reconciliation, not two
expenses. Missing quantities or missing tariffs remain unknown. Arithmetic is
exact and unit-specific; no implicit currency conversion is introduced.

The selected producer and original emission identity matter. Copying annotated
data downstream is not a second expense. Retry Held/Consumed/Published refer
to the original attempts, not new consumption. Conversely, two actual attempts
can incur two charges even if the parent only receives the final response.
This selection/deduplication policy must be qualified with Retry and lineage
before any total is presented as authoritative.

### Proposed pre-effect protocol

The proposed owner is the App Graph. Its account may represent a session,
tenant or shared deployment; the attachment point does not choose the account
scope, and neither rule is frozen yet. Its available quantity is
the limit minus settled spending and outstanding reservations, all in the
same unit. Limits and price policies are deployment inputs, not values frozen
in graph source.

1. Prepare the exact outgoing request and resolve its account, policy and
   charging evidence. Derive a reservation ceiling under a versioned policy.
2. Reserve atomically against that authority. On refusal, start no Driver;
   the eventual typed refusal and its routing need a separate design decision.
3. Commit the authorization before external work. Each actual Retry attempt
   requires its own admission; concurrent instances cannot independently read
   the same available balance and both spend it.
4. Settle against the original outcome evidence, release unused reservation
   when justified, and retain both the decision and its evidence for recovery.
   An authorized correlated refund may credit an account; an arbitrary
   negative annotated field may not.

Illustrative arithmetic, not a measurement: with a USD 10 limit, USD 8 settled
and USD 1 reserved, only USD 1 is available. A Call requiring USD 2 must refuse
even though a dashboard shows only USD 8 spent. A USD 1 reservation later
settled at USD 0.60 releases USD 0.40, without inventing a refund transaction.

A strict monetary ceiling requires a trustworthy upper bound on the pending
expense, not merely its eventual measurement. Request/output limits and a
versioned provider tariff may supply such a bound only where the Contract and
charging rules actually guarantee it. Otherwise policy must refuse strict
admission or explicitly offer an estimated ceiling; it cannot promise that a
post-call observation prevents an overrun that already occurred.

A timeout or a cancelled Session does not prove that the provider charged
nothing. Unknown settlement must remain a visible unresolved liability. Whether
policy holds a reservation until reconciliation or conservatively debits the
ceiling is still open. The latter is a budget-policy charge, not evidence that
the provider actually billed that amount. Silence cannot release money by
default, and stopping a Retry does not erase earlier attempts.

### Durability, telemetry and remaining decisions

Reservation, authorization and settlement must survive the recovery horizon
claimed by the budget. M1's in-memory journal and diagnostic recording cannot
promise a shared balance surviving crashes or multiple Workers. Durable
journaling is M2; its existence alone will not supply the atomic shared-account
protocol, external reconciliation or distributed admission. These need their
own qualification before a cross-run guarantee is advertised.

The future `otlp` observer can export consumption, estimated/reported cost and
budget state once their producers exist, under explicit mapping and privacy
rules. It does not supply tariffs or enforce a limit. Collector availability,
aggregation delay, rounding and export loss cannot control admission or replace
the exact evidence. The debugger remains an inspector, not the account lock.

Before implementation, arbitrate: the first account scope and authority;
guaranteed versus estimated reservation; source binding and refusal semantics;
unknown outcomes and refunds; nested/shared budgets; durable recovery and
Retry/Closing composition. The `Budget` name and Extension category are
settled, but no attributes, executable tag or record are reserved by this
update. The honest M1 promise is measurement evidence, not an enforced
monetary ceiling.

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
11. A `PriceBook` is versioned deployment input. Every quote or
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
