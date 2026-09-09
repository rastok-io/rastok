# Reference simulation 1 — session router with a timeout path

> **Status: historical pre-v16 derivation, not a current golden or program.**
> Process service/correlation remains deferred; the abbreviated identity and
> timing formulas below also predate current instance/run records. Preserve
> them as design evidence, not byte-exact conformance. Use the
> [current examples](../README.md), [identity rules](../../spec/identity.md)
> and [execution model](../../spec/execution-model.md) for current behavior.

## 1. What this sim exercises (the checklist)

| Rule | Source |
|---|---|
| Root id derivation at inbound (`blake3(connector ‖ inbound_seq)`) | envelope.md §2 |
| Emission id derivation (`node_path ‖ trigger ‖ parent ‖ port ‖ index`) | envelope.md §2, execution-model.md §4 |
| Edge = same envelope, same id | envelope.md §3 |
| Mailbox drain to stable state between fixture steps | execution-model.md §8 |
| Per-call `timeout`, armed at delivery, matched by the Connector's correlator | execution-model.md §6.1 |
| Expiry = `TimeoutExpired` on typed `exit`, trigger `l:timeout`, parent = request | execution-model.md §6.2 |
| The call-closed rule : late response discarded-and-journaled | execution-model.md §6.3 |
| `on_timeout = bypass` (default) : the instance lives | execution-model.md §6.4 |
| `advance` = discrete sweep (dated events in date order, zero-time computation) | execution-model.md §8 |
| Fixture seeds via `meta.ext` (never `meta.sys` — A2) | envelope.md §6 |
| Rastok-expression guard on typed pinned input; absent-field grid row 1 | grammar.md, expression references |
| Per-hop journal entries: arrival seq, logical instance address, committed_at | envelope.md §7 |

## 2. The graph

```rastok
graph sim01 : App {

  connector teams : HttpServer {
    path   = "/api/messages"
    effect = none                       // pure source
  }

  connector codex : Process {
    command    = "codex"
    args       = ["app-server"]
    mode       = service
    codec      = jsonl
    correlate  = data.request_id
    effect     = idempotent
    behavior   = probabilistic
    timeout    = duration("10s")                    // per-call budget (§6.1)
    on_timeout = bypass                 // default, written for the reader
  }

  operator route {
    instance = { key = meta.ext.user_id }
    lifetime = { kind = idle; after = duration("2h") }

    input in : struct { kind: text, text: text }     // typed pin — grid row 1
    var quota : int = 100

    on in where data.kind == "message" && quota > 0 {
      quota = quota - 1
      out = {
        meta.app.route = "codex"
        data = {
          request_id = string(meta.sys.id)
          op = "turn/start"
          prompt = data.text
        }
      }
    }
  }

  operator triage {
    input in : ProcessExit               // reacts to selected exit cases from codex
    on in where data is TimeoutExpired {
      out = {
        data = { kind = "apology", text = "The agent timed out. Retrying soon." }
      }
    }
  }

  teams -> route -> codex.in
  codex.exit -> triage
  { codex.out ; triage.out } -> teams.in
}
```

## 3. The fixture

```rastok
lab sim01 {
  simulation sim01_session_router on app {
    scenario timeout_path "one message, codex never answers, apology goes back" {

      mock codex {
        on in where data.op == "turn/start" { out = nothing }   // deliberately never replies
      }                                                          // → the Connector's timeout machinery works

      teams.out = {
        meta.ext.user_id = "alice"        // the world's half seeds meta.ext — A2
        data = { kind = "message", text = "Bonjour" }
      }

      expect "turn started" codex.in where data.op == "turn/start"

      advance duration("30s")

      expect "apology delivered" teams.in where data.kind == "apology"
      expect nothing codex.out                                   // the ghost check, positively assertable
    }
  }
}
```

## 4. The expected trace — derived by hand

Symbolic ids (`H` = blake3 truncated to 128 bits). Virtual clock starts at
`t = 0`. `inbound_seq` for `teams` starts at 0 (fixture-owned).

### Step 1 — injection (`teams.out = { … }`), then drain

```
t=0  E0 born at inbound
       id        = H( "teams" ‖ 0 )                      ← root recipe
       parent_id = ∅          origin_id = E0.id           role = default
       graph_version = 1      ext.content_type = absent (structured value)

     mailbox session=alice ◄─ E0 (arrival seq 0 in this box — journaled)

     TRANSACTION route (trigger p:in, parent E0)
       guard : data.kind == "message" && quota > 0        → true (typed pin, bare access legal)
       state : quota 100 → 99                              (committed atomically with emissions)
       emits E1 on out (index 0) :
         id        = H( "route" ‖ "p:in" ‖ E0.id ‖ "out" ‖ 0 )
         parent_id = E0.id     origin_id = E0.id

     edge route -> codex.in : SAME envelope E1 (no new id — edges are wires)

     delivery of E1 to codex.in :
       the supervision plane arms the call chrono : expires at t = 0 + 10s   ← dated event #1
       the mock matches (when data.op == "turn/start") and answers… nothing (timeout action)

     all mailboxes empty → stable state.                   fixture step complete
```

Journal so far (per-hop): `[E0 arrival @route[key="alice"] seq0 committed_at=0]`,
`[E1 emitted by route]`, `[E1 delivered @codex, chrono armed t=10]`.

### Step 2 — `expect "turn started" codex.in where …`

Evaluated on the stable state : the last envelope delivered to `codex.in` is
E1, `data.op == "turn/start"` → **pass**. (Zero virtual time consumed.)

### Step 3 — `advance duration("30s")` (target t = 30)

```
sweep :
  mailboxes empty ✓ → earliest dated event ≤ 30 ?  YES : t=10, call-timeout of E1
    t := 10 ; the supervision journals the decision and delivers :
    E2 on codex.exit :
       id        = H( "codex" ‖ "l:timeout" ‖ E1.id ‖ "exit" ‖ 0 )
       parent_id = E1.id      origin_id = E0.id     role = default
       tagged case = TimeoutExpired
    on_timeout = bypass → the codex instance LIVES (no terminal envelope, no restart)
    THE CALL IS CLOSED : E1's correlation entry is retired (§6.3)

    drain : mailbox (triage is unkeyed → its singleton box) ◄─ E2
      TRANSACTION triage (trigger p:in, parent E2)
        guard : data is TimeoutExpired → true
        emits E3 on out (index 0) :
          id        = H( "triage" ‖ "p:in" ‖ E2.id ‖ "out" ‖ 0 )
          parent_id = E2.id    origin_id = E0.id
      edge triage.out -> teams.in : E3 delivered to the outbound Connector
    mailboxes empty → stable.

  earliest dated event ≤ 30 ?  NONE → t := 30. done.
```

The lineage of the apology reads end-to-end :
`E3 → E2 (l:timeout) → E1 → E0` — one unbroken parent chain from the apology
back to the inbound message, crossing the supervision plane without a hole.

### Step 4 — the final expectations

Stable state at t=30, window opened by `advance` : E3 delivered to `teams.in`,
`data.kind == "apology"` → **pass** ; zero deliveries at `codex.out` in the
window → `expect nothing` **pass** (the call-closed rule, asserted positively).

### Ghost check (the call-closed rule)

Variant of the fixture : the mock replies at `t = 15` (after expiry). Expected :
the Connector's correlator finds E1's call already closed → the reply is
**discarded and journaled** (`late response, discarded`) — it never enters any
mailbox, no envelope is born, the trace above is byte-identical, and
`expect nothing codex.out` asserts it positively.
