# Reference simulation 6 — HttpServer `respond` : lineage correlation, 504, the orphan answer

> **Status : Reference derivation — not yet executable.** The expected trace
> is derived by hand from
> [stdlib.md](../../spec/stdlib.md) §3.1, [execution-model.md](../../spec/execution-model.md)
> §6/§8 and [delivery-semantics.md](../../spec/delivery-semantics.md) §5.
> It is a conformance target for `respond` correlation, expiry and overload.

## 1. What this sim exercises (the checklist)

| Rule | Source |
|---|---|
| `mode = respond` : the request stays open ; the graph's answer completes it | stdlib.md §3.1 |
| Correlation **default = lineage** (`origin_id == R.id`), legal on join-free return paths | stdlib.md §3.1 (Correlator) |
| `timeout` = the open-request budget ; expiry → `RequestExpired`, 504, `l:timeout`, call-closed | stdlib.md §3.1 (Error set) |
| A late graph answer = **orphan**, discarded-and-journaled, never two HTTP responses | stdlib.md §3.1 |
| Unwired `err` bubbles lexically to `app.err` | delivery-semantics.md §5 |
| The pending-connection table is volatile ; response delivery is best-effort | HttpServer adapter obligation |
| Mock `after <duration>` = dated emission, swept by `advance` | simulation-semantics.md §5.1 |
| `expect nothing` / `count == N` windows | simulation-semantics.md §5.2 |
| Open requests are **bounded** → immediate 503, `Overloaded` journaled ; the engaged are protected | stdlib.md §3.1, delivery-semantics.md §2 |
| A test-scale bound must produce the same real refusal path as production | open conformance boundary (§5) |

## 2. The graph

```rastok
graph sim06 : App {

  connector api : HttpServer {
    path    = "/api/ask"
    mode    = respond
    timeout = duration("5s")            // the open-request budget (504 deadline)
  }

  connector llm : HttpClient {
    base_url = env("LLM_URL")
    path     = "/v1/answer"
    effect   = idempotent
    behavior = probabilistic
    timeout  = duration("8s")
  }

  operator ask {
    input in : struct { q: text }
    on in { out = { prompt = data.q } }
  }

  operator wrap {
    input in : struct { text: text }
    on in { out = { answer = data.text } }
  }

  api -> ask -> llm.in
  llm.out -> wrap -> api.in             // join-free return path → default lineage correlation LEGAL
  // llm.err deliberately unwired → bubbles to app.err (delivery-semantics §5)
}
```

Compile-time check exercised : the return path `llm.out -> wrap -> api.in`
crosses **no root-mixing join**, so the default lineage correlator is
accepted without an explicit `correlate` (stdlib §3.1 — the compile error
fires only when a join sits on the path).

## 3. The lab

```rastok
lab sim06 {
  simulation sim06_http_respond on app {

    scenario answered "the graph answers within the budget ; the open request completes" {
      mock llm {
        on in {
          after duration("1s")
          out = { text = "42" }
        }
      }

      api.out = {
        meta.ext.request_id = "r-1"
        data = { q = "meaning of life?" }
      }

      expect "llm called" llm.in where data.prompt == "meaning of life?"

      advance duration("2s")

      expect "answer reached the endpoint" api.in where data.answer == "42"
      expect nothing app.err
    }

    scenario expired "the graph answers too late ; 504 first, the orphan is discarded" {
      mock llm {
        on in {
          after duration("6s")                 // > the 5s open-request budget
          out = { text = "too late" }
        }
      }

      api.out = {
        meta.ext.request_id = "r-2"
        data = { q = "slow one" }
      }

      advance duration("10s")

      expect "504 path taken" app.err where err.kind == "RequestExpired"
      expect "the graph still answered" wrap.out count == 1 where data.answer == "too late"
      expect "exactly one completion attempt" api.in count == 1
      // the DISCARD of that one late delivery is asserted by the golden trace,
      // not by an expect.
    }

    scenario newsletter_rush "two served, the third politely refused (Overloaded/503)" {
      mock api { max_open_requests = 2 }        // driver settings on an ingress mock —
                                                // prod : thousands ; here : the testable scale
      mock llm {
        on in {
          after duration("4s")                  // a slow answer keeps requests OPEN
          out = { text = "the linen one" }
        }
      }

      // three clients click within the same second (the business shape :
      // a shop's « help me choose » button, the evening a newsletter lands)
      api.out = { meta.ext.visitor = "v1"  data = { q = "summer jacket" } }
      api.out = { meta.ext.visitor = "v2"  data = { q = "office shoes" } }
      api.out = { meta.ext.visitor = "v3"  data = { q = "rain coat" } }

      // the refusal is IMMEDIATE — zero virtual time has passed
      expect "the third hit the bound" app.err where err.kind == "Overloaded"
      expect nothing llm.in                     // a refusal at the door : v3 never entered the graph

      advance duration("5s")

      expect "both engaged clients served" api.in count == 2 where data.answer == "the linen one"
      expect nothing app.err                    // no 504 : the bound protected the engaged
    }
  }
}
```

The ingress mock block is intended to carry **driver settings only, bounds
only** — the fixture still does the injecting. In the
lowering the mock IS the lab's adapter, and the pending table lives in the
adapter, so its cap is declared where it lives. The machine's
refusal is a real mechanism that must be *produced*, never scripted — the
mocks supply the world (arrivals, slowness), the bound supplies the
testable scale, and the 503 that comes out is earned by the real pending
table, exactly as sim-01's `TimeoutExpired` is earned by scripted silence
+ `advance`, never recited.

## 4. The expected trace — derived by hand

Symbolic ids (`H` = blake3 truncated to 128 bits). Virtual clock starts at
`t = 0` per scenario ; `inbound_seq` for `api` starts at 0 (fixture-owned).

### Scenario `answered`

```
t=0  R0 born at inbound (ack after durable INBOUND)
       id = H( "api" ‖ 0 )    parent = ∅    origin = R0.id
     the driver's pending-connection table gains { R0.id → open connection }
     the open-request budget arms a dated event : expires t = 5     ← dated #1

     TRANSACTION ask (p:in, parent R0) → E1 on out (index 0)
       E1.id = H( "ask" ‖ "p:in" ‖ R0.id ‖ "out" ‖ 0 )   origin = R0.id
     delivery E1 → llm.in : OUTBOUND open (key, digest), chrono t = 8  ← dated #2
     mock matches ; `after 1s` → scripted closure due t = 1           ← dated #3
     mailboxes empty → stable.                    window ① complete

expect "llm called" : E1 delivered to llm.in, data.prompt matches → PASS

advance duration("2s")                        (target t = 2 ; window ②)
  t=1  OUTBOUND closed (mock reply) → E2 on llm.out (p:in, parent E1)
         E2.id = H( "llm" ‖ "p:in" ‖ E1.id ‖ "out" ‖ 0 )   origin = R0.id
       TRANSACTION wrap (p:in, parent E2) → E3 on out
         E3.id = H( "wrap" ‖ "p:in" ‖ E2.id ‖ "out" ‖ 0 )  origin = R0.id
       edge wrap -> api.in : E3 delivered to the endpoint node
         the driver correlates : E3.origin_id == R0.id, pending table hit
         → HTTP 200 leaves on the open connection (driver plane, best-effort)
         → pending entry retired ; the request's budget timer (dated #1)
           is DISARMED (call closed by completion)
  no dated event ≤ 2 remains → t := 2. stable.

expect "answer reached the endpoint" : E3 at api.in in window ② → PASS
expect nothing app.err : zero deliveries → PASS
```

Lineage of the answer : `E3 → E2 → E1 → R0` — one unbroken origin chain,
which is exactly what the default correlator reads.

### Scenario `expired`

```
t=0  R0' born (id = H("api" ‖ 0) — fresh scenario, fresh seq)
     pending table { R0'.id → open } ; budget timer due t = 5        ← dated #1
     TRANSACTION ask → E1' ; OUTBOUND open llm, chrono t = 8         ← dated #2
     mock matches ; closure due t = 6                                ← dated #3
     stable.                                        window ①

advance duration("10s")                       (target t = 10 ; window ②)
  t=5  budget expiry fires (l:timeout — VM-owned dated event) :
         · the driver answers 504 on the open connection ; pending entry retired
         · THE REQUEST IS CLOSED (call-closed rule, stdlib §3.1)
         · typed err on api.err :
             X.id = H( "api" ‖ "l:timeout" ‖ R0'.id ‖ "err" ‖ 0 )
             err kind = RequestExpired    parent = R0'   origin = R0'.id
         · api.err unwired → X bubbles to app.err (delivery-semantics §5)
       stable at t=5.
  t=6  OUTBOUND closed (mock reply) → E2' on llm.out (the LLM call itself
       was within ITS 8s budget — dated #2 disarmed by closure)
       TRANSACTION wrap → E3' on out (count : wrap.out fired once)
       edge wrap -> api.in : E3' delivered to the endpoint node
         the driver correlates : origin R0'.id → pending table MISS
         (the 504 already went out) → ORPHAN : discarded-and-journaled
         (stdlib §3.1 — « never two HTTP responses ») ; no envelope is born
       stable.
  t=8 disarmed ; nothing ≤ 10 remains → t := 10.

expect "504 path taken"    : X at app.err, err.kind == RequestExpired → PASS
expect "the graph still answered" : E3' on wrap.out, count == 1 → PASS
expect "exactly one completion attempt" : E3' at api.in, count == 1 → PASS
   (the delivery HAPPENED in the graph plane ; the discard is the driver's,
    and the golden trace records it)
```

### Scenario `newsletter_rush`

```
t=0  action 1 : R1 born (seq 0) — accepted : pending table {R1} (1/2)
       budget timer due t=5 ; TRANSACTION ask → E1 ; OUTBOUND open llm
       (chrono t=8) ; mock closure due t=4.   stable → window ①
     action 2 : R2 born (seq 1) — accepted : pending {R1,R2} (2/2 : FULL)
       same machinery → E2, second open, closure due t=4.   window ②
     action 3 : v3's request arrives — pending table FULL :
       · REFUSED AT THE DOOR : the 503 leaves immediately (driver plane)
       · NO INBOUND record is written — the request was never accepted,
         so the acknowledgement discipline has nothing to promise
       · root typed err X on api.err : kind = Overloaded (no parent —
         the same posture as MalformedRequest) → bubbles to app.err
       stable → window ③

expect "the third hit the bound" (window ③) : X, kind Overloaded → PASS
expect nothing llm.in (window ③) : zero deliveries — v3 never entered → PASS

advance duration("5s")                                  (window ④)
  t=4  both scripted closures fire : llm.out replies → wrap → api.in ×2
       both correlate (pending hits R1, R2) → two 200s leave,
       both budget timers (t=5) DISARMED by completion
  t=5  nothing armed remains → t := 5.

expect "both engaged clients served" : two deliveries at api.in,
  data.answer == "the linen one", count == 2 → PASS
expect nothing app.err (window ④) : X was window ③ ; no 504 → PASS
   — the bound protected the engaged : v1/v2 completed inside their
   budget, indifferent to the crush.
```

## 5. Conformance boundary

The `answered` and `expired` scenarios use the normative public surface.
`newsletter_rush` remains an **unsealed design vector** until two contracts
are specified publicly :

1. a bounds-only override for an ingress mock, without scripting the refusal ;
2. the root-id recipe for an `Overloaded` error emitted before an INBOUND
   record is accepted.

Until then, that scenario must not be blessed as a golden trace. Its
port-visible expectations still state the intended overload behavior.
