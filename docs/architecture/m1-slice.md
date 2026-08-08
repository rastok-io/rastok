# The M1 slice — the public-opening milestone

> Implementation slice — non-normative. Opened 2026-08-06, target 15
> October 2026: the repository goes public with this milestone, and
> its closing demonstration is **a complete flow running without the
> real systems** — compiled, simulated against mocks, golden-traced,
> watched live.

## The shape of M1

M0 proved the loop; M0.5 consolidated the engine
([exit state](m0.5-slice.md)). M1 widens the executable subset — and
the discipline is already in place for exactly this: the checker
refuses every not-yet-executable construct **by name**
(`E-CHECK-DEFERRED`), so the milestone is a chosen list of deferrals
lifted one by one. Lifting one deferral means: its frozen-spec
section becomes executable, its tests and scenarios land, its golden
projection evolves only by a deliberate re-bless, and everything not
lifted keeps refusing by name. The semantic hash will move when the
language grows — each move is pinned deliberately, never observed
after the fact.

## Scope, in order

The order is load-bearing: the multiport machine comes first because
joins, `per`, and the HTTP pair all stand on it — and its
delivery-reference prerequisite was sealed in M0.5 precisely so this
step touches `vm.rs` once.

1. **Multiport execution** — handlers on named ports, multiple
   outputs, typed ports; the OUTBOUND key already carries the
   canonical edge, so call identity is ready for it. One transport
   consequence to honor from day one: when a connector contract
   exposes non-default ports, the plan must carry the RESOLVED port
   table — the embedded validator has no `Registry` to consult, so
   everything it checks must travel in the package.
   **LANDED 2026-08-08** — named ingress (`1d5d1a3`: INBOUND key 24,
   D-2026-08-08-2) and pinned port types (`fa861d8`: `port_decl`'s
   optional type, E-PORT-TYPE, D-2026-08-08-3); routing and named
   outputs were already alive, proved by the lift's probes. Held to
   review the same day and re-closed on four findings (`92da490`
   ingress is not observable, `c8459f9` ONE conformance notion over
   every EmitValue form and the physical int bound, `0b0b963` the
   resolved contract table ships from day one with the registry
   seam, `cf5d21c` format v3 with the gauge fixture pinning hash,
   package, and trace; `8b61739` third pass — the checker and the
   machine share ONE data-construction model: the `{…}` shorthand
   builds the struct the grammar defines, builders fold their writes
   in the runtime's order, constructed content is judged at the pin
   at emission-build time, and the Registry validates sheets at the
   door and is never consulted past resolution; `2aa50f4` fourth
   pass — the frontiers: boundary outputs carry their port, named
   connector edges defer by name until the Partner seam speaks
   them, one pre-commit choke point judges every emission — synthesized
   errors included — against every pin of its fan-out,
   the builders are physically STRUCTS (tag 17) across all three
   interpreters, and the validator audits pinned edges). The trio's
   own contract types join the connector-sheet items below.
2. **The `jsonl` codec** — structured payloads across the `Process`
   boundary, under the one byte discipline; unlocks realistic `File`
   and HTTP payloads.
3. **The error half of the language** — typed-error emissions from
   operators (`err = Kind {…}`) and mock `err` replies in the Lab.
   Referees can then provoke connector failures without adapters;
   the benchmark's task set grows the same way.
4. **`per` — the partition surface** — keyed nodes and graphs on the
   flat two-concept model; scheduler keys over the proven multiport
   routing.
5. **Joins — `on (a, b) within … / last`** — correlation windows;
   the last language lift of the milestone. Identity note, decided
   with the handler canonicalization: the join's activation KEY must
   preserve its declared port order — `on (a, b)` and `on (b, a)`
   are different identities (identity.md makes port order inside a
   join sensitive), while the order BETWEEN handlers stays
   insensitive and canonically sorted by that key.
6. **`File` connector** — the first new adapter on the conformance
   harness: the half-day claim gets its first real measurement, with
   no network in the way.
7. **`HttpClient`** — outbound calls under declared budgets, on the
   harness. Decision point before code: hand-rolled bounded HTTP/1.1
   versus a vetted dependency, and whether TLS enters M1 at all (the
   October demo runs mocked; a `decisions.md` entry either way).
8. **`HttpServer`, ack mode** — the inbound side: ingress authority,
   submission sequence, backpressure at the boundary.
9. **The Loop aspect** — the bounded-iteration primitive, alive on a
   graph.
10. **Cost counters** — the `cost` map (stdlib §6.4) accumulated
    from the journal; a subscriber, not a core dependency.
11. **`--watch`** — the HTML+SSE mini-visualization, strictly a
    journal subscriber; the observation plane stays outside the
    core by construction.
12. **Usage review and the example gallery** — continuous, seeded by
    the benchmark's twenty reference solutions, closed by the
    October demo flow: one realistic end-to-end graph (ingest →
    decide → act → journal) running entirely against mocks.

## Minimal evidence

- each lifted deferral: the `E-CHECK-DEFERRED` site deleted, the
  spec section referenced from the change, scenario coverage in the
  Lab, and — where bytes move — a deliberately re-blessed golden
  with its `decisions.md` entry;
- the whole gate green throughout, semantic-hash moves pinned;
- `File`, `HttpClient`, `HttpServer` each landing with a conformance
  profile — three data points on the half-day claim;
- the gallery running green under `rastok simulate`, the demo flow
  among it;
- the benchmark re-run cheap by construction: tasks and referees
  extended alongside each language lift, prompt card v2 issued at
  the end of the milestone (v1 stays frozen for the stability
  measurement).

## Explicit non-goals

No durable journal on disk (the format is designed, not built), no
Saga manager, no `compensable`/`irreversible` execution, no
`Process` service mode, no distribution, no cluster, no registry, no
TLS commitment before its decision entry. The parallelism decisions
of [`parallelism-notes.md`](parallelism-notes.md) stay a decision
list — M1 remains one worker.
