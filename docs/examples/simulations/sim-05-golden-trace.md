# Reference simulation 5 — fixture → golden trace, derived by hand

> **Status : Reference derivation — not yet executable.** This document
> derives the complete golden trace of a small scenario by hand so a
> conforming runtime has a byte-exact target. Canonicalization follows
> [identity.md](../../spec/identity.md), while the role of golden traces is
> defined in
> [simulation-semantics.md](../../spec/simulation-semantics.md) §6.2.

## 1. What this sim exercises

| Rule | Source |
|---|---|
| All three record types in one run (INBOUND / TRANSACTION / OUTBOUND) | delivery §6 |
| Canonical order = the deterministic execution order of the sweep | execution-model §8, sim-semantics §6.1 |
| Virtual `committed_at` (no wall-clock anywhere) | sim-semantics §6.1 (5) |
| Maps serialize sorted by key | sim-semantics §6.1 (6) |
| Same-shape contract : the mocked call's OUTBOUND records | sim-semantics §2 |
| The fingerprint : blake3 over the canonical bytes | sim-semantics §6 |
| Cursor advances visible in transaction records | delivery §2 |

## 2. The graph — three record types, minimal surface

```rastok
graph sim05 : App {
  durability = local
  partition user { key = meta.app.user_id; lifetime = duration("2h") }

  connector ping   : HttpServer { path = "/ping" ; effect = none }
  connector notify : HttpClient {
    url    = "https://hooks.example/notify"
    effect = idempotent
    behavior = deterministic
    timeout = duration("5s")
  }

  operator count per user {
    input in : struct { }
    var n : int = 0
    on in {
      n = n + 1
      out = { meta.app.seen = n, data = { n = n } }
    }
  }

  ping -> count -> notify.in
  notify.out -> nothing            // the reply is not routed further — written, not silent
}
```

## 3. The scenario

```rastok
lab sim05 {
  simulation sim05_golden on app {
    scenario two_pings "alice pings twice ; every record accounted for" {
      mock notify {
        on in { out = { ok = true } }
      }
      ping.out = { meta.app.user_id = "alice", data = { } }
      ping.out = { meta.app.user_id = "alice", data = { } }
      expect "counted twice" notify.in count == 1 where data.n == 2
    }
  }
}
```

*(The window rule is per action step : the `expect` reads the SECOND
injection's window — which contains exactly one delivery at `notify.in`, the
`n == 2` emission. The assertion holds under the strict rule ; no
"action block" notion exists.)*

## 4. THE GOLDEN TRACE — the complete journal, in canonical order

Symbolic ids : `H(...)` = blake3/128 ; `d(...)` = request digest. Virtual
clock : injections at `t=0` (each drains to stable before the next — the
records interleave in **execution order**, the journal's total order).
In the rendering below, `∅` means that the canonical key is omitted, never
encoded as null. `committed_at` and arming time `t` are shown as annotations
for readability and are excluded from the causal-core fingerprint.

```
#  record      content (canonical field order ; maps sorted by key)
── ─────────── ────────────────────────────────────────────────────────────────
1  INBOUND     { connector: "ping", inbound_seq: 0, committed_at: 0,
                 envelope: { id: A0 = H("ping"‖0), parent: ∅, origin: A0,
                             role: default, graph_version: 1,
                             meta.app: { user_id: "alice" },      ← sorted keys
                             data: struct {} } }
2  TRANSACTION { node: "count", key: "alice", trigger: "p:in",
                 consumed: [ (edge ping→count, pos 1) ],          ← the cursor
                 state: { n: 0 → 1 },
                 emissions: [ (out, 0, C1 = H("count"‖"p:in"‖A0‖"out"‖0),
                               { meta.app: { seen: 1, user_id: "alice" },
                                 data: { n: 1 } }) ],
                 committed_at: 0 }
3  OUTBOUND      { open : call C1, connector: "notify",
                 key: ∅ (idempotent, no idempotency_key), digest: d(C1), t: 0 }
                 — chrono armed : would expire at t=5
4  OUTBOUND      { closed(response) : call C1, reply R1 = { ok: true } }
5  TRANSACTION { node: "notify", key: "alice", trigger: "p:in",
                 consumed: [ (edge count→notify, pos 1) ],
                 state: ∅,
                 emissions: [ (out, 0, N1 = H("notify"‖"p:in"‖C1‖"out"‖0),
                               { data: { ok: true } }) ],
                 committed_at: 0 }
                 — N1 wired to `nothing` : discarded, WRITTEN in the graph text
6  INBOUND     { connector: "ping", inbound_seq: 1, committed_at: 0,
                 envelope: { id: A1 = H("ping"‖1), … } }
7  TRANSACTION { node: "count", key: "alice", trigger: "p:in",
                 consumed: [ (edge ping→count, pos 2) ],
                 state: { n: 1 → 2 },
                 emissions: [ (out, 0, C2 = H("count"‖"p:in"‖A1‖"out"‖0),
                               { meta.app: { seen: 2, user_id: "alice" },
                                 data: { n: 2 } }) ],
                 committed_at: 0 }
8  OUTBOUND      { open : call C2, connector: "notify", digest: d(C2), t: 0 }
9  OUTBOUND      { closed(response) : call C2, reply R2 = { ok: true } }
10 TRANSACTION { node: "notify", key: "alice", trigger: "p:in",
                 consumed: [ (edge count→notify, pos 2) ],
                 emissions: [ (out, 0, N2 = H(…‖C2‖"out"‖0), { data: { ok: true } }) ],
                 committed_at: 0 }
── ─────────── ────────────────────────────────────────────────────────────────
GOLDEN FINGERPRINT = blake3( canonical causal-core bytes of records 1..10 )
```

The expectation (`count == 1 where data.n == 2` at `notify.in`, window =
second injection) : one delivery matches (C2) → **pass**.

## 5. The byte-discipline, stated on this trace

1. **Total order = execution order** — the deterministic order of the sweep
   (drain-to-stable per step ; within a drain : mailbox FIFO, text-order
   emissions). Two runs give records 1..10 in this exact order, always.
2. **Time is annotation** — every displayed `committed_at` above is `0`
   because both injections occur at virtual t=0. Time annotations may exist
   in storage but are excluded from the canonical causal core.
3. **Sorted maps** — `meta.app` serializes `{ seen, user_id }` (u > s ? no :
   's' < 'u' → `seen` first) ; unordered in memory, ordered on the wire,
   always.
4. **Canonical form is distinct from storage** — the golden projects each
   record through identity.md's envelope spine and tagged-value rules.
   Absent fields are omitted ; durable journal bytes may use another
   representation.
5. **The same-shape contract, visible** : records 3-4 and 8-9 exist although
   `notify` was mocked — the mock replaced the partner, not the bookkeeping.
6. **Nothing else** : no spans, no durations, no host names — the parallel
   plane leaves no fingerprint.

An OUTBOUND record without `idempotency_key` is legal for
`effect = idempotent`; its canonical key is omitted, not encoded as null.
