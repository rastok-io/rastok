# Reference simulation 7 — Calendar : cron + tz, the two DST edges, `origin`

> **Status : Reference derivation — not yet executable.** The expected trace
> is derived by hand from
> [stdlib.md](../../spec/stdlib.md) §3.5 (the DST policy, stated) and
> [simulation-semantics.md](../../spec/simulation-semantics.md) §5.3
> (`origin`). It is a conformance target for cron expansion across both DST
> edges.

## 1. What this sim exercises (the checklist)

| Rule | Source |
|---|---|
| `cron + tz` mandatory together — never the host's implicit zone | stdlib.md §3.5 |
| Ticks = INBOUND records, payload `struct { scheduled_at: timestamp }` | stdlib.md §3.5 |
| **Spring forward : a nonexistent civil time is SKIPPED** | stdlib.md §3.5 (DST policy) |
| **Fall back : an ambiguous civil time fires ONCE, first occurrence** | stdlib.md §3.5 (DST policy) |
| `origin` : virtual t=0 ≡ a declared civil instant ; absent = epoch + warning | simulation-semantics.md §5.3 |
| Calendar declares no runtime failure case — bad cron/tz is a compile refusal | stdlib.md §3.5 |
| `Memory` bounded store receives the ticks' descendants | stdlib.md §3.6 |

The civil facts (IANA `Europe/Paris`, year 2026) : spring forward on
**Sunday 2026-03-29** (02:00 CET → 03:00 CEST ; civil times 02:00–02:59 do
not exist) ; fall back on **Sunday 2026-10-25** (03:00 CEST → 02:00 CET ;
civil times 02:00–02:59 occur twice — first as CEST/+02:00, then as
CET/+01:00).

## 2. The graph

```rastok
graph sim07 : App {

  connector nightly : Calendar {
    cron = "30 2 * * *"                 // 02:30, every civil day…
    tz   = "Europe/Paris"               // …in this zone — the dangerous slot, on purpose
  }

  connector log : Memory { cap = 100 }

  operator stamp {
    input in : struct { scheduled_at: timestamp }
    on in { out = { fired_at = data.scheduled_at } }
  }

  nightly -> stamp -> log
}
```

## 3. The lab

```rastok
lab sim07 {
  simulation sim07_calendar_dst on app {

    scenario spring_forward "02:30 on the skipped night never fires" {
      origin = timestamp("2026-03-28T00:00:00+01:00")     // Saturday, midnight Paris (CET)

      advance duration("48h")

      expect "exactly one tick in two days" log.in count == 1
      expect "and it is Saturday's" log.in
        where data.fired_at == timestamp("2026-03-28T02:30:00+01:00")
    }

    scenario fall_back "02:30 on the ambiguous night fires once, first occurrence" {
      origin = timestamp("2026-10-24T00:00:00+02:00")     // Saturday, midnight Paris (CEST)

      advance duration("48h")

      expect "two ticks in two days" log.in count == 2
      expect "the ambiguous one fired exactly once, as CEST" log.in count == 1
        where data.fired_at == timestamp("2026-10-25T02:30:00+02:00")
    }
  }
}
```

A third scenario is deliberately *not* written : a `Calendar` scenario with
no `origin` compiles with a **warning** (simulation-semantics §5.3 — « your
cron fires at instants you did not choose ») ; that diagnostic belongs to
the negative/warning corpus, not to a golden-traced scenario.

## 4. The expected trace — derived by hand

Ticks are INBOUND roots : `id = H( "nightly" ‖ inbound_seq )`, payload
`scheduled_at`. The lab driver expands `cron + tz + origin` into dated
events **in physical (virtual) time** ; all civil→physical conversions below
use IANA `Europe/Paris`. The tzdata version is a run-affecting binding and
must be pinned through the boot attestation defined by identity.md ; otherwise
the same scenario could expand differently after a timezone-rule change.

### Scenario `spring_forward`  (origin ≡ 2026-03-27T23:00:00Z)

```
cron expansion over [t=0, t=48h] :
  Sat 2026-03-28 02:30 CET   = 01:30Z  → virtual t =  2h30    ← dated #1
  Sun 2026-03-29 02:30       — DOES NOT EXIST (02:00→03:00 jump)
                              → SKIPPED (stdlib §3.5) ; nothing armed
  Mon 2026-03-30 02:30 CEST  = 00:30Z  → virtual t = 49h30    — beyond window

advance duration("48h") :
  t=2h30  T0 born at inbound : id = H("nightly" ‖ 0)
            data.scheduled_at = 2026-03-28T02:30:00+01:00
          TRANSACTION stamp (p:in, parent T0) → E1 on out
          delivery E1 → log.in (Memory write, within cap)
          stable.
  no further dated event ≤ 48h → t := 48h.

expect count == 1 : exactly T0's descendant at log.in → PASS
expect Saturday's instant : data.fired_at == 2026-03-28T02:30+01:00 → PASS
   (timestamp equality is instant-based : +01:00 form ≡ 01:30Z)
```

The night of the 29th leaves **no trace at all** — no INBOUND, no dated
event, no error : a skipped nonexistent time is silence, not a failure
(Calendar declares no runtime failure case).

### Scenario `fall_back`  (origin ≡ 2026-10-23T22:00:00Z)

```
cron expansion over [t=0, t=48h] :
  Sat 2026-10-24 02:30 CEST  = 00:30Z          → virtual t =  2h30   ← dated #1
  Sun 2026-10-25 02:30       — AMBIGUOUS (occurs at 00:30Z as +02:00,
                               then again at 01:30Z as +01:00)
                              → fires ONCE, FIRST occurrence (stdlib §3.5)
                              = 2026-10-25T02:30:00+02:00 = 00:30Z
                                               → virtual t = 26h30   ← dated #2
  Mon 2026-10-26 02:30 CET   = 01:30Z          → virtual t = 51h30   — beyond window

advance duration("48h") :
  t=2h30   T0 (seq 0) : scheduled_at = 2026-10-24T02:30:00+02:00
           stamp → log.in.        stable.
  t=26h30  T1 (seq 1) : scheduled_at = 2026-10-25T02:30:00+02:00
           stamp → log.in.        stable.
  nothing ≤ 48h remains → t := 48h.

expect count == 2 : T0's and T1's descendants at log.in → PASS
expect the ambiguous one, once, as CEST :
  exactly one delivery with fired_at ≡ 2026-10-25T02:30+02:00 → PASS
  (had the policy fired both occurrences, count would be 2 for that instant’s
   civil label and 3 for the window — both expects would catch it)
```

The expectations rely on Rastok timestamp equality being instant-based
(offset-insensitive), so the author can write the civil representation while
asserting the instant.
