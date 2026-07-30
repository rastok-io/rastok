# Reference simulation 3 — Loop retry × outbound WAL × key stability

> **Status : Reference derivation — not yet executable.** The expected trace
> is derived by hand from
> [delivery-semantics.md](../../spec/delivery-semantics.md) §4 and
> [loop.md](../../spec/loop.md). It is a conformance target for the outbound
> record lifecycle, the call-closed rule and stable idempotency keys.

## 1. What this sim exercises

| Rule | Source |
|---|---|
| Outbound record : `open` (key, digest, t) before the call | delivery-semantics §4.2 |
| One closure : response XOR timeout ; late response → `discarded` | execution-model §6.3, delivery §4.2 |
| `TimeoutExpired` on `err` (l:timeout, parent = the request) | execution-model §6.2 |
| Loop re-injection : `l:next`, parent = the previous call's output | loop.md §1, envelope §3.1 |
| **Stable business key across attempts → partner dedups** | delivery §4.3 |
| Recovery triage : `open` + key present → recall byte-same key | delivery §4.4 |
| Request digest = integrity seal on retry | delivery §4.2 |
| `Loop.backoff` = a dated event in the sweep | loop.md §4, execution-model §5/§8 |

## 2. The graph

```rastok
graph sim03 : App {
  durability = local
  partition order { key = meta.app.order_id; lifetime = duration("30m") }

  connector orders : HttpServer { path = "/orders" ; effect = none }
  connector ledger : Memory { }

  connector pay : HttpClient + Loop {
    url             = "https://bank.example/charge"
    effect          = irreversible
    behavior        = deterministic
    idempotency_key = meta.app.payment_id            // STABLE business key — the §4.3 norm
    timeout         = duration("10s")
    Loop.variant = measure {
      initial = 3
      next = value - 1
      until = value <= 0
    }
    Loop.backoff = constant { delay = duration("5s") }
    on err where !err.transient                       // base case : permanent err
    on err where err.transient { }                    // iterating case (TimeoutExpired is transient)
  }

  orders -> pay.in
  pay.out -> ledger.in
  pay.err -> dlq.in
  connector dlq : Memory { }
}
```

## 3. The fixture

```rastok
lab sim03 {
  simulation sim03_loop_retry on app {
    scenario slow_bank "the bank executes but answers late ; retry must NOT double-pay" {
      mock pay {
        // Stripe-semantics mock : the FIRST key use executes ; a REUSED key
        // replies instantly with the ORIGINAL outcome (partner-side dedup).
        on in where meta.sys.iter_count == 0 {
          after duration("12s")
          out = { status = "charged", charge_id = "ch_1" }
        }
        on in { out = { status = "charged", charge_id = "ch_1", deduplicated = true } }
      }

      orders.out = envelope {
        meta.app.order_id   = "42"
        meta.app.payment_id = "pay_42"
        data = struct { amount = 99.90 }
      }
      advance duration("30s")
      expect "charged exactly once" ledger.in count == 1 where (
        data.status == "charged" && data.charge_id == "ch_1")
    }
  }
}
```

## 4. The expected trace — derived by hand

```
t=0    E0 born (orders, seq 0) ; delivered to pay.in
       journal : INBOUND(E0) · OUTBOUND open { call E0, key "pay_42", digest d0, t=0 }
       chrono armed : expires t=10                                 ← dated event

advance duration("30s") (target 30) :
  earliest ≤ 30 ?  t=10 : call-timeout of E0
    t := 10 ; OUTBOUND(E0) := closed(timeout) — THE CALL IS CLOSED
    E1 on pay.err : id = H("pay" ‖ "l:timeout" ‖ E0.id ‖ "err" ‖ 0)
       err kind = TimeoutExpired (transient)
    drain : Loop matches `on err where err.transient` → re-injection scheduled
       AFTER backoff : dated event at t = 10 + 5 = 15
       (E1 also wired to dlq — it lands there : the timeout is visible, by design)

  earliest ≤ 30 ?  t=12 : the mock's LATE reply to call E0
    t := 12 ; OUTBOUND(E0) is closed → reply DISCARDED and journaled
    (« late response, discarded » — observability-visible, graph-invisible.
     The bank HAS charged ch_1. The graph does not know it. This is the trap.)

  earliest ≤ 30 ?  t=15 : the backoff event — Loop re-injects
    t := 15 ; E2 : id = H("pay" ‖ "l:next" ‖ E1.id ‖ … ), iter_count = 1
    delivered to pay.in
    journal : OUTBOUND open { call E2, key "pay_42", digest d0, t=15 }
                                    ▲ STABLE — same operation, same key
    mock (dedup branch) replies instantly :
      OUTBOUND(E2) := closed(response) ; pay commits its transaction record
      E3 on pay.out (trigger p:in, parent E2) : status "charged", ch_1
    drain : ledger stores E3. Loop exits on success (implicit base case).

  earliest ≤ 30 ?  none → t := 30. done.

`expect … count == 1` → pass.  ONE charge (ch_1), TWO attempts, ZERO double-pay — the count quantifier makes « exactly once » a real assertion.
```

### The counterfactual — why the stable-key rule is required

Same fixture, but `idempotency_key = meta.app.payment_id + "#" +
string(meta.sys.iter_count)` :

```
   attempt #0 : key "pay_42#0" → the bank charges ch_1 (reply lost to timeout)
   attempt #1 : key "pay_42#1" → a NEW key → the bank sees a NEW operation
                → charges ch_2.       💸💸  DOUBLE PAYMENT.
```

The per-attempt key remains available as the **documented escape** for
partners that consume the key on a processed failure ; it must not be the
default. *(delivery-semantics §4.3.)*

### Recovery derivation — the §4.4 triage, row « open + key »

This additional derivation is not a step of the executable scenario above.
Crash right after `OUTBOUND open { call E0, key "pay_42" }`, before any reply :

```
   restart (durability = local) :
     fold : E0 delivered, OUTBOUND(E0) open, no closure → unknown outcome
     triage : key present → RECALL with the byte-same re-derived key "pay_42"
              (digest re-check : d0 == d0 ✓ — the graph didn't change under the call)
     the bank : first use of "pay_42" → charges once ; or dedups if the lost
     call had landed. EITHER WAY : exactly one charge.
```

Because `pay.err` is wired to the DLQ, the timeout attempt remains visible
even when the Loop later succeeds. That DLQ records *attempts* ; the final
outcome lives on `pay.out`.
