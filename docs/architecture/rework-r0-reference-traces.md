# R0 reference traces — Loop r2 by host

> Hand-written semantic traces, frozen 2026-09-05. These are review fixtures,
> not claims about the then-current v15 output bytes. `T` denotes a TRANSACTION record,
> `I` an INBOUND record, and `O` an OUTBOUND record. Bracketed annotations such
> as due time are deliberately outside the canonical causal seal.

## Notation

```text
DR(x)    ordinary DeliveryRef x
AR(x)    opening-admission reference x
LR(x)    LoopRunRef derived from owner instance + AR(x)
CR(x)    ManagerCommandRef
LC[...]  active LoopContext stack
Q?       quiescence test; an observation, not a journal record
```

Each `Produced` item is part of the producer's transaction. Each `Consumed`
item is part of a later manager transaction. A crash between them is therefore
recoverable without replaying the producer.

### R9 executable correspondence — 2026-09-07

The frozen sketches below omit ordinary lifecycle records and have two
record-granularity inaccuracies; they are not byte-for-byte journal goldens:

- `LC[r0]` on an opening route names the eventual application. The executable
  journal does not claim that context before the manager has committed Open
  and Admit; it derives it at admission from the retained opening reference.
- Graph trace rows 3–4 and Call trace rows 3–4 are **one OUTBOUND open** in
  the implementation, not an additional request-consuming TRANSACTION followed
  by open. Preparation is pure; the committed open retains and claims the
  request. Its cursor is consumed once, in the completion transaction after
  the correlated closure. No effect starts before the open commit.

The authorities are `Vm::open_loop_application`, `Vm::admit_graph_loop` and
the outbound delivery/completion path; executable prefix comparisons live in
`v16_loop.rs` and `v16_recovery.rs`, including
`call_recovery_restores_the_held_delivery_without_restarting_the_driver`.
These corrections change the illustrative mapping, not the v16 record format,
effect order, or command-produced/command-consumed crash window.

## 1. Operator — two applications, natural Done

Source:

```rastok
operator countdown : Loop {
  input in: struct { n: int }
  output out: text
  Loop.fuel = 2
  on in match {
    data.n <= 0 => out = "liftoff"
    _           => Loop.next = { n = data.n - 1 }
  }
}
```

Opening value is `{ n = 1 }`. The App route to `countdown.in` has delivery
reference `DR(d0)` and opens `LR(r0)`.

| # | record / fold | semantic result |
|---:|---|---|
| 0 | `I root e0; route DR(d0) -> countdown.in; LC[LR(r0)]` | target capacity and Loop admission gate reserved; `AR(d0)` is durable |
| 1 | `T manager Open { r0, AR(d0), entry=in, fuel=2, state=e0 }` | run exists with iteration 0, remaining 2 |
| 2 | `T manager Admit { r0, iteration=0, admission=DR(d0) }` | retained delivery becomes consumable |
| 3 | `T countdown consumes DR(d0); Produced CR(c0)=Next(e1{n=0})` | Dispatch and command commit atomically; application 0 complete |
| — | `Q?(r0) = true` | Operator has no other obligation |
| 4 | `T manager Consumed(c0); Advance { r0, 0->1, remaining=1, state=e1 }` | no delay, no destination held between records |
| 5 | `T manager Admit { r0, iteration=1, admission=AR(a1) }` | new delivery on the remembered `in` is committed |
| 6 | `T countdown consumes a1; emits out=e2; Produced CR(c1)=Done(implicit)` | terminal output and Done are one producer commit |
| — | `Q?(r0) = true` | application 1 quiescent |
| 7 | `T manager Consumed(c1); Close { r0, iteration=1, Natural }` | run closes; App-visible `out` remains ordinary routing |

Fuel counts exactly records 3 and 6, the two Operator applications. No spatial
edge points backward to `in`.

## 2. Graph — quiescence, final recurrence, and `Loop.expired`

Source shape:

```rastok
graph refinement : Loop {
  input in: Draft
  output out: Answer
  Loop.fuel = 1

  operator audit {
    input in: HttpClientExit
    on in { nothing }
  }

  in -> model -> judge
  model.exit -> audit
  judge.accepted -> out
  judge.revised -> Loop.next
}

refinement.out -> publish
refinement.Loop.expired -> failed
```

The model produces a revised Draft. Fuel permits the current application but no
second application.

| # | record / fold | semantic result |
|---:|---|---|
| 0 | `T upstream emits e0; route DR(d0) -> refinement.in; LC[LR(r0)]` | exact crossing of the Loop Graph boundary is retained |
| 1 | `T manager Open { r0, AR(d0), entry=in, fuel=1, state=e0 }` | Graph itself executes no handler |
| 2 | `T manager Admit { r0, iteration=0, admission=DR(d0) }; route -> model.in` | manager expands the boundary route after capacity proof |
| 3 | `T model consumes internal delivery; seals request e1` | Connector transaction |
| 4 | `O CallOpen(call0, digest(e1))` | Driver may start only now |
| — | `Q?(r0) = false` | call0 is an open contained obligation |
| 5 | `O CallClosed(call0, response)` | correlated outcome, not an INBOUND root |
| 6 | `T model completion; emits model.out=e2 and model.exit=e3` | internal deliveries retained under `LC[r0]` |
| — | `Q?(r0) = false` | judge delivery remains pending |
| 7 | `T judge consumes e2; emits revised=e4; Produced CR(c0)=Next(e4)` | command exists, but model.exit/audit work may remain |
| — | `Q?(r0) = false` | a sibling audit delivery still belongs to r0 |
| 8 | `T audit consumes sibling delivery` | last contained obligation closes |
| — | `Q?(r0) = true` | manager may consume c0 |
| 9 | `T manager Consumed(c0); Expired { r0, iteration=0, last_state=e4, output=e5 }`; route `e5 -> failed` | no `Advance`, no `l:next`, no re-admission; output capacity proved before commit |

The Graph's terminal `out` was not selected. `Loop.expired` is productible
because explicit fuel can be exhausted and therefore requires the parent route.
The output is produced by the manager, not by the Graph.

## 3. Connector Call — `Again`, crash, recovery, then natural Done

Source shape:

```rastok
connector pay : HttpClient + Loop {
  effect = idempotent
  behavior = probabilistic
  Loop.fuel = 2
  Loop.delay = { kind = fixed ; after = duration("1s") }
  // other required HttpClient settings
}

pay.exit -> match {
  TimeoutExpired -> pay.Loop.again
  _              -> nothing
}
pay.out -> done
pay.Loop.expired -> failed
```

| # | record / fold | semantic result |
|---:|---|---|
| 0 | `T caller emits request e0; route DR(d0) -> pay.in; LC[LR(r0)]` | exact opening admission retained |
| 1 | `T manager Open { r0, AR(d0), entry=in, fuel=2, state=e0 }` | run opens |
| 2 | `T manager Admit { r0, iteration=0, admission=DR(d0) }` | request becomes consumable |
| 3 | `T pay consumes d0; prepares request` | Connector application begins |
| 4 | `O CallOpen(call0, digest(e0))` | effect begins only after record 3 committed |
| 5 | `O CallClosed(call0, TimeoutExpired)` | terminal observation correlates to call0 |
| 6 | `T pay completion; emits exit=e1; Produced CR(c0)=Again(cause=e1)` | Call closes and recurrence verdict commits atomically |
| — | `Q?(r0) = true` | no frame, delivery, or Call remains |
| **crash** | **after #6, before manager consumption** | Driver is not called again during recovery |
| R1 | recovery folds owners, cursor d0, closed call0, and `Produced(c0)-Consumed(∅)` | requeues c0 exactly once |
| 7 | `T manager Consumed(c0); Advance { r0, 0->1, remaining=1, state=e0 }`; timer kind 5 | current state retained; due instant is annotation |
| 8 | `T manager Admit { r0, iteration=1, admission=AR(a1) }` at due time | capacities resolved now, not during the wait |
| 9 | `T pay consumes a1; prepares byte-identical logical request` | new call identity, same business state |
| 10 | `O CallOpen(call1, digest(e0))` | permitted because effect is idempotent |
| 11 | `O CallClosed(call1, Completed)` | terminal response retained |
| 12 | `T pay completion; emits out=e2, exit=e3; no recurrence route selected; Produced CR(c1)=Done(implicit)` | frame routes to App; exit supplies natural terminal verdict |
| — | `Q?(r0) = true` | call1 closed and routes committed |
| 13 | `T manager Consumed(c1); Close { r0, iteration=1, Natural }` | run closes |

If application 1 had selected `Again`, record 13 would instead be `Expired`
and emit `Loop.expired`; it would never open call2. If `effect` were
`compensable` or `irreversible`, the route from the `TimeoutExpired`
`outcome_unknown` class to Again would fail at check with `E-LOOP-EFFECT`.

## Cross-trace checks

- Each recurrence returns to the exact opening entry port.
- No command is a spatial edge or port.
- Every command is durable before manager consumption.
- Fuel counts applications, not records or output frames.
- Quiescence is per `LoopRunRef`; work from another run cannot block these
  traces.
- `Loop.expired` is reserved and routed before its producing manager record
  commits.
- Recovery replays no producer transaction and starts no effect speculatively.
