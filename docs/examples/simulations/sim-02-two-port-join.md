# Reference simulation 2 — two-port join, with a `JoinExpired` eviction

> **Status: executable join semantics, reference-only Drivers.** M1.4 R6
> executes the presence, FIFO, instance-local correlation and typed-expiry
> rules below. The complete scenario remains a paper derivation until its
> `HttpServer` and `Memory` Drivers land; the public executable proof lives in
> `crates/rastok/tests/instances_lab.rs`.

## 1. What this sim exercises (the checklist)

| Rule | Source |
|---|---|
| Join `on (a, b)`: early arrival buffered in the Operator instance | execution-model.md §3 |
| The triggering envelope = the presence-completing arrival (mailbox FIFO) | execution-model.md §3 |
| Join emission : parent = the trigger, trigger = the trigger's port | envelope.md §3 |
| Explicit namespace attribution on join output | envelope.md §3 |
| `within duration("15m")` : the eviction chrono as a dated event | execution-model.md §5/§7 |
| Eviction of a join buffer = `JoinExpired` on typed `expired` (l:evict, parent = evictee) | execution-model.md §7 |
| Window vs join boundary: this outcome is JOIN-only | execution-model.md §7/§9 |
| Two keys = two mailboxes = independent lifecycles | execution-model.md §2 |
| DLQ wiring of the expiry path (never silent) | philosophy + `Memory` connector |

## 2. The graph

```rastok
type JoinExpiry = union { JoinExpired {} }

graph sim02 : App {

  connector orders   : HttpServer { path = "/orders"   ; effect = none }
  connector payments : HttpServer { path = "/payments" ; effect = none }
  connector ledger   : Memory { }                       // settled results
  connector dlq      : Memory { }                       // evicted halves — never silent

  operator settle {
    instance = { key = meta.ext.order_id }
    lifetime = { kind = idle; after = duration("30m") }

    input order   : struct { total: decimal(10,2) }
    input payment : struct { amount: decimal(10,2) }
    output expired: JoinExpiry

    on (order, payment) within duration("15m") {
      out = {
        meta.app.order_ref = order.meta.app.order_ref   // join : explicit pick ; common fields merge (D-2026-08-10-2)
        data = {
          amount = payment.data.amount
          status = "settled"
        }
      }
    }
  }

  orders   -> settle.order
  payments -> settle.payment
  settle.out -> ledger.in
  settle.expired -> dlq.in                              // JoinExpired lands here, written not silent
}
```

## 3. The fixture

```rastok
lab sim02 {
  simulation sim02_two_port_join on app {
    scenario join_and_eviction "order 42 settles ; order 43's payment never comes" {

      // ---- case A : the happy join -----------------------------------------
      orders.out = {
        meta.ext.order_id = "42"
        data = { total = 99.90 }
      }
      payments.out = {
        meta.ext.order_id = "42"
        data = { amount = 99.90 }
      }
      expect "42 settled" ledger.in where data.status == "settled" && data.amount == 99.90

      // ---- case B : the abandoned half --------------------------------------
      orders.out = {
        meta.ext.order_id = "43"
        data = { total = 10.00 }
      }

      advance duration("15m")

      expect "43's order evicted, loudly" dlq.in where data is JoinExpired
    }
  }
}
```

## 4. The expected trace — derived by hand

Symbolic ids (`H` = blake3/128). `t = 0` at start ; `inbound_seq` counters :
`orders` → 0,1… ; `payments` → 0…

### Case A — the happy join

```
t=0  A0 born at inbound (orders)          id = H("orders" ‖ 0)     origin = A0
     mailbox settle[key="42"] ◄─ A0 (arrival seq 0 — address journaled)
       TRANSACTION attempt : presence(order, payment) incomplete (payment missing)
       → A0 BUFFERED in the settle instance ; within chrono ARMED : evicts A0 at t=15min   ← dated event
     mailboxes empty → stable.

t=0  P0 born at inbound (payments)        id = H("payments" ‖ 0)   origin = P0
     mailbox settle[key="42"] ◄─ P0 (arrival seq 1)
       presence complete → TRANSACTION settle
         TRIGGER = P0 (the presence-completing arrival, FIFO)      → trigger port = p:payment
         parent  = P0
         A0's within chrono is CLEARED (its buffered envelope is consumed)
         emits S0 on out (index 0) :
           id        = H( "settle" ‖ "p:payment" ‖ P0.id ‖ "out" ‖ 0 )
           parent_id = P0.id        origin_id = P0.origin (= P0.id)
           meta.app  = A0's meta.app  (explicit attribution — order's namespace)
     edge settle.out -> ledger.in : S0 delivered.
     stable. `expect "42 settled"` (window : the payments.out injection) → pass
```

Provenance note (the 3-piece model, envelope.md §7) : S0's **parent chain**
leads to P0 ; A0's contribution is recorded in the **per-hop journal** (join
contribution at this transaction). The full DAG is journal-reconstructable ; the
envelope carries the primary lineage only. Both `origin_id`s (A0's, P0's) are
distinct roots — the join output inherits its *trigger's* origin ; the other
root remains reachable via the journal. One OTel trace therefore follows the
trigger side.

### Case B — the abandoned half

```
t=0   B0 born at inbound (orders, seq 1)   id = H("orders" ‖ 1)    origin = B0
      mailbox settle[key="43"] ◄─ B0 (arrival seq 0 in THIS instance — independent key and mailbox)
        presence incomplete → B0 BUFFERED ; within chrono armed : evicts at t=15min   ← dated event
      stable.

advance duration("15m") (target t=15min) :
  earliest dated event ≤ 15min ? YES : t=15min, eviction of B0's buffer
    t := 15min ; the supervision journals the eviction and delivers :
    X0 on settle.expired :
       id        = H( "settle" ‖ "l:evict" ‖ B0.id ‖ "expired" ‖ 0 )
       parent_id = B0.id        origin_id = B0.id      role = default
       tagged case = JoinExpired
    (this is a JOIN-buffer eviction → expired. A closing WINDOW would have
     emitted its aggregate via on stop instead — the §7/§9 boundary.)

    drain : dlq.in ◄─ X0 (Memory stores it — the loss is explicit and visible)
    stable.
  earliest ≤ 15min ? none → t := 15min. done.

`expect "43's order evicted, loudly"` (window : the advance) → pass
```

The lost fact has a full paper trail : `X0 → B0`, key "43", evicted at
t=15min (`committed_at` in the journal) — auditable, compensatable, never
silent.

### Cross-checks

- **Two keys, two boxes** : order=42's flow and order=43's flow never
  interact ; their relative order is undefined and unobservable — conforming.
- **The trigger rule under FIFO** : if the fixture had sent P0 *before* A0,
  the trigger would be A0 (`p:order`), the parent A0, the id different —
  *demonstrating* that join outputs are journal-relative (the arrival order is
  a race in prod, fixed by the fixture in sim). Exactly envelope.md §2.0.
