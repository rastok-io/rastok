# Reference simulation 4 — saga compensation × crash recovery (the fold)

> **Status : Reference derivation — not yet executable.** The expected trace
> is derived by hand from
> [delivery-semantics.md](../../spec/delivery-semantics.md) §5-§7 and
> [effects-and-sagas.md](../../spec/effects-and-sagas.md). It is a
> conformance target for err bubbling, the state fold, recovery during
> rollback and idempotent compensation replay.

## 1. What this sim exercises

| Rule | Source |
|---|---|
| Unwired `err` bubbles into the enclosing `: Saga` = the compensation trigger | delivery §5 |
| Saga state = a fold over the scope's records (no 4th record type) | delivery §7.1 |
| `l:compensate` : parent = the failing envelope, emitter = the `: Saga` node | envelope §3.1 |
| Crash mid-rollback → fold + outbound triage + resume at j+1 | delivery §7.2 |
| Compensation carries its OWN key (`…/release`), idempotent on replay | delivery §4.3, effects §3.2 (`E-COMPENSATE-KEY`) |
| `Saga.timeout` re-armed from the creator record's `committed_at` | delivery §7.2 |
| `none` not in the saga durability domain (static check) | delivery §7.3 |

## 2. The graph

```rastok
graph sim04 : App {
  durability = local                       // a file — the saga domain's floor
  partition booking { key = meta.app.booking_id; lifetime = duration("1h") }

  connector bookings : HttpServer { path = "/book" ; effect = none }
  connector confirmed : Memory { }

  graph book_trip : Saga {
    Saga.timeout = duration("10m")

    connector reserve : HttpClient {
      url    = "https://hotel.example/hold"
      effect = compensable
      behavior = deterministic
      idempotency_key = meta.app.booking_id + "/hold"
      on compensate {
        idempotency_key = meta.app.booking_id + "/release"   // its OWN operation (E-COMPENSATE-KEY)
        out = { data = { op = "release" } }
      }
    }
    connector pay : HttpClient {
      url    = "https://bank.example/charge"
      effect = irreversible
      behavior = deterministic
      idempotency_key = meta.app.booking_id + "/charge"
    }
    . -> reserve -> pay -> .
    // pay.err is UNWIRED on purpose : it bubbles into `: Saga` — the trigger.
  }

  bookings -> book_trip -> confirmed.in
}
```

*(Static check exercised by construction : replacing `durability = local`
with `none` makes this app **fail to compile** — `E-SAGA-DURABILITY`,
« `none` is not a durability of a saga ». The sim runs on `local`.)*

## 3. The fixture

```rastok
lab sim04 {
  simulation sim04_saga_recovery on app {
    scenario declined_then_crash "reserve holds, pay declines, crash mid-rollback, recovery finishes it" {
      mock reserve {
        on in where data.op == "release" { out = { status = "released" } }
        on in                            { out = { status = "held" } }
      }
      mock pay {
        on in { err = PaymentDeclined { transient = false } }
      }

      bookings.out = {
        meta.ext.booking_id = "bk_7"
        data = { room = "12", amount = 250.00 }
      }
      crash after reserve.outbound_open#2        // the 2nd outbound open of reserve = the compensation's
      restart
      expect "saga rolled back, loudly" app.err where err.kind == "SagaRolledBack"
      //   (the window rule : restart is an ACTION step — the recovery deliveries,
      //    including the saga's terminal SagaRolledBack emission, are ITS window)
    }
  }
}
```

## 4. The expected trace — derived by hand

### Forward, then the decline

```
t=0  E0 born (bookings) ; enters book_trip → saga instance created
     journal : INBOUND(E0) · TRANSACTION (instance-creating — committed_at = 0)
                                 ▲ the fold will read « open » from HERE ;
                                   Saga.timeout budget runs from 0 to 10min
     reserve : OUTBOUND open { key "bk_7/hold" } → mock replies "held"
               → OUTBOUND closed · TRANSACTION (reserve sealed)      → E1 on out
     pay     : E1 delivered ; OUTBOUND open { key "bk_7/charge" }
               → mock replies err PaymentDeclined (transient = false)
               → OUTBOUND closed · TRANSACTION : E2 on pay.err
                  err kind = PaymentDeclined, parent = E1

     pay.err is UNWIRED → E2 BUBBLES : pay → book_trip, intercepted by `: Saga`
     fold so far : open, 1 step sealed (reserve), permanent failure seen
                 → rolling_back, 0/1 compensations done

     the saga node emits the compensation envelope :
       E3 : id = H("book_trip" ‖ "l:compensate" ‖ E2.id ‖ … )
            parent = E2 (the failing envelope), role = compensate
       delivered to reserve.in → on compensate handler runs
       OUTBOUND open { call E3, key "bk_7/release" (its OWN key — E-COMPENSATE-KEY), digest d3 }
```

### 💥 crash — exactly here (after `open`, before any reply)

### Recovery (durability = local — the journal survived)

```
restart :
  ① FOLD the journal of scope book_trip/bk_7 :
       instance-creating tx @ t=0            → open
       reserve sealed                        → 1 step
       E2 err bubbled into the saga          → rolling_back
       l:compensate E3 emitted               → compensation 1 in flight
     state : « rolling_back, 0/1 acked »           ← computed, not stored
  ② OUTBOUND triage : open(E3), no closure → unknown outcome
       compensation target must be idempotent (static rule, effects-and-sagas §3.2)
       → RECALL with the byte-same key "bk_7/release" (digest d3 re-checked ✓)
       mock : "released" → OUTBOUND closed · TRANSACTION (compensation acked)
  ③ resume the fold : 1/1 compensations acked → rolled_back
       the saga emits its terminal typed err SagaRolledBack { saga_id } —
       it bubbles to the app root's default sink : the §5 bubble ends LOUDLY.
  ④ Saga.timeout : re-armed for the REMAINING budget
       (opened at committed_at = 0 ; 10min budget ; recovery at journal-time —
        the chrono did not restart from zero, it resumed)
```

The expectation reads the `restart` step's window (the recovery deliveries) at
`app.err` : **pass** — rolled back, exactly one hold, exactly one
release, zero charges kept.
