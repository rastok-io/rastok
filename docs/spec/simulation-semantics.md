# Simulation Semantics — scenarios, mocks, golden traces

> **Status: Normative — under review.** The `lab` is the home of
> simulations and experiments;
> `theorem` is a reserved declaration (§10–§11).
> Companion to [execution-model.md](execution-model.md) (virtual time —
> `advance` is defined there, §8) and
> [delivery-semantics.md](delivery-semantics.md) (the journal this doc
> scripts and fingerprints).

The question this doc answers : **how do you specify, exercise and freeze an
app's behavior — before and without touching the real world ?**


## 1. Vocabulary — one word per layer

| Word | Layer | Meaning |
|---|---|---|
| **`lab`** | the App's twin | the never-deployed corpus bound to the homonymous `graph <name> : App`: its simulations, experiments, and future theorems. Fragments with the same name merge across the unit's files; it never alters the production graph or its `graph_version` |
| **feature** (Gherkin) | intent | optional business front-end (§8) — never the executable truth |
| **`simulation`** | possibility space | the executable space of possible worlds around **one closed graph** (`on <graph>` — `on app` today). It groups shared mocks/prelude and named scenarios |
| **`experiment`** | field evidence | the real App run under an external deployment `profile`, with at least one real Connector boundary, in real time (§10) |
| **`scenario`** | region / trajectory | a named slice of that space ; when run, one deterministic trajectory : fixture steps + expectations |
| **fixture** | inside a scenario | **not a first-rank object** — the pre-written outer trace of a Scenario (injections, mock replies, time advances, faults) : the scripted **world-half of the journal** (§2) |
| **expectations** | inside a scenario | the assertions (§5) |
| **`theorem`** *(reserved)* | universal property | a declarative statement over ALL admissible trajectories of (graph + simulation), linked by `over <simulation>` (§11) — never "run a few times" |
| **the simulator** | executor | the deterministic runner (virtual clock, mocks, scripted journal) — one, in the runtime |
| **golden trace** | artifact | the run's journal, canonically serialized, byte-frozen (§6) |

The ontology, side by side — production on the left, its twin on the right :

| Production | Lab twin |
|---|---|
| App | Lab |
| Graph | Simulation (closes it) |
| Connector | Mock (replaces the partner, §5) |
| deployed App + real profile | Experiment |
| a concrete execution | Scenario |
| an observation | Expect |
| Graph + Simulation | ⊨ Theorem (future) |
| Operator | **itself** — pure logic is never mocked : it runs |

```rastok
lab checkout {                           // the App's twin — never deployed
  simulation payments on app {           // one closed graph, one possibility space
    mock bank { … }                        // shared by the scenarios below
    scenario happy_path    { … }
    scenario declined_card { … }
    scenario bank_timeout  { … }
  }
}
```

The mirror is deliberate :

| Graph side | Simulation side | Reading |
|---|---|---|
| graph | simulation | the declared machine vs. its possibility space |
| Connector boundary | mock | possible behavior of the external partner |
| source Connector output | injection | the world sends an envelope into the graph |
| handler guard | expectation predicate | the graph reacts vs. the scenario observes |
| runtime world | fixture | the recorded world-half, written in advance |

Read it as a mirror rather than as a separate test framework : the graph
declares the machine ; the simulation declares the worlds that can surround it.
Shared mocks define common Connector behavior ; each scenario selects a
concrete slice of inputs, time, faults and expectations. Running that scenario
yields one deterministic trace.


## 2. The fixture IS the journal's world-half

The journal-relative contract says `run = f(graph, journal)`, and the journal
has two halves : what **the graph** did (transaction records) and what **the
world** did (inbound, dated supervision decisions, outbound outcomes). In
production the world-half is *recorded* ; in simulation there is no world —
**the fixture plays its role, written in advance** :

| Fixture step | Plays | Journal record produced |
|---|---|---|
| `orders.out = { … }` | the world sends | **INBOUND** (fixture-owned `inbound_seq`) |
| `advance duration("30s")` | time passes | the dated decisions (the sweep, execution-model §8) |
| `mock … { on in … }` | the partner answers | the **OUTBOUND closures** |
| `crash after <record>` / `restart` | the process dies | volatile truncation + recovery |
| `expect …` | *(nothing — pure observation)* | *(none)* |

**The same-shape contract** — what makes golden traces mean anything :

> A simulated run produces a journal of the **same shape** as a production
> run. A mocked call still writes its real OUTBOUND records (`open` with key
> and digest, `closed` with the mock's reply) ; the reply enters through the
> Connector's normal response path (trigger `p:in`, parent = the request).
> The mock replaces the partner, **not the bookkeeping**.

Two-mode equivalence, stated journal-relative : *same world-half in →
byte-identical **causal cores** out*
(delivery-semantics §1 : a record = causal core + observation annotations ;
`committed_at` — wall-clock in production, virtual in simulation — is an
annotation, outside the comparison). This makes the claim **checkable against
production** : fingerprint a prod slice's causal cores and compare them to
the bridge-generated scenario's.

**Boundaries** :
- **Scope = the lab of the deployed unit** : simulations live in the
  **`lab`** whose name
  matches the unit's single `: App` graph (grammar.md, "Unit formation" +
  `lab_decl`), and each names the graph it closes : **`on app`** — the only
  executable target today. A sub-graph target (`on payment`) parses and
  type-checks but is **deferred (E-SIM-TARGET)** : closing a sub-graph
  requires a *birth recipe* for envelopes entering its public ports (they
  are born mid-lineage, with no inbound Connector and no `inbound_seq`) —
  exactly what mid-graph injection prohibition protects. Until that recipe
  is specified, wrap the sub-graph in a thin App. Node references in mocks,
  injections and expectations resolve in the **unit's namespace** —
  top-level declarations by name, and the App root's own children directly
  (the root's name is excluded, as in `node_path` —
  [envelope.md](envelope.md)) : `mock outbound` (a root child),
  `mock sessions.codex` (a top-level sibling),
  `expect city_weather.build_request.out`.
- **Injection at inbound only** : envelopes are injected at source-Connector
  outputs (`orders.out = …`) — born through the root recipe, clean lineage.
  Mid-graph injection is a **compile error** (it would fabricate an envelope
  with no lineage and no id recipe). To unit-test one operator, wrap it in a
  three-line app with a `Memory`/`HttpServer` source.
- **Observation anywhere** : `expect` may target any port, read-only.
- **Steps run sequentially, each drains to a stable state**
  (execution-model §8) — an expectation never reads an in-between.


## 3. Syntax — the graph's own grammar, mirrored

A simulation is *"writing the graph's possible worlds, in the graph's
language"*. Almost every simulation construct is an existing graph
construct :

```rastok
// INJECTION = the emission syntax (an inbound emission, scripted)
orders.out = { meta.ext.user_id = "alice", data = { kind = "message" } }

// MOCK = a scripted stand-in whose body is HANDLER grammar — nothing new
mock bank {
  on in where meta.sys.iter_count == 0 {
    after duration("12s")                                       // ← the ONE addition : dated emission
    out = { data = { status = "charged" } }
  }
  on in { out = { status = "charged", deduplicated = true } }   // §4 : a data record
}
// never answer  = { out = nothing }   → the Connector's `timeout` machinery works
// typed outcome = { exit = PaymentDeclined { reason = "policy" } }

// EXPECTATION = a guard head, read in the mirror
expect ledger.in where data.status == "settled"
//  on   ledger.in where …   → the graph REACTS
//  expect ledger.in where … → the scenario ASSERTS
```

Only ~six words are simulation-specific : `simulation`, `scenario`, `mock`,
`expect`, `advance`, `after`. Everything else — `on`, `where`, `out =`,
ordinary port assignments, `nothing`, the braced builder, Rastok
expressions — is the language the developer
(and the generating LLM) already speaks.


## 4. The braced value — `{ … }`, classified once

In **any envelope position** (handler emissions, fixture injections, mock
replies) a braced value is classified ONCE by its top-level roots
(D-2026-08-09-1) :

- **No reserved root** → a DATA RECORD : `{ status = "charged" }` builds a
  fresh `struct` payload (replacing `data` entirely) ; heritable metadata
  stays the input's (inbound defaults in fixtures).
- **Only reserved roots** (`data`, `meta`) → the ENVELOPE
  form : `{ meta.ext.user_id = "alice", data = { kind = "message" } }`
  patches or builds the envelope itself ; a whole-`data` assignment makes
  the data fresh (`out = { data = "hello", meta.ext.content_type = "text/plain" }`
  is the non-struct spelling).
- **A mix** → `E-BRACE-MIXED`, never a silent reinterpretation. Business
  fields named `data` or `meta` take the explicit escape
  `data = { … }`.

`{}` is the empty record ; the exact re-emission of the current envelope
is `.` (`out = .`).


## 5. Mocks and expectations — the fine rules

### 5.1 Mocks

- **Two levels, nearest wins** : a `mock` at `simulation` level is shared by
  its scenarios ; a scenario-level `mock` for the same Connector **replaces**
  it (per Connector, whole-mock replacement — no branch merging).
- **First match in text order** ; a bare `on in { … }` is the catch-all.
- **An uncovered outbound call fails the scenario, loudly** — a call reaching a
  mock with no matching branch, or a Connector with no mock at all, prints
  the request (node, key, digest, data) and fails. Wanting silence is
  *written* : `mock bank { on in { out = nothing } }`. No guessed defaults —
  this is what keeps failure-space coverage (§7) honest.
- **Every mock output is compile-checked** against the resolved type of
  that Connector port. One mock completion may emit several ports
  atomically, exactly like the real boundary.
- **Deterministic and stateless** : a mock is a pure function of the
  request envelope ; `after <duration>` joins the dated-events queue (swept
  by `advance`) ; `meta.sys.iter_count` covers retry branching (sim 3). A
  per-mock invocation counter is a noted open point, admitted only if a real
  case demands it.
- Sources need no mock (the fixture injects) ; only calling Connectors do.

### 5.2 Expectations

**The window rule** : an *action step* (injection, `advance`,
`crash`/`restart`) opens a window ; the `expect` lines that follow read **the
deliveries of that window**, without consuming it :

```rastok
orders.out = { … }               // ACTION → window ①
expect pay.in where …            // reads window ①
expect nothing pay.exit          // reads the SAME window ①
advance duration("30s")                      // ACTION → window ②
expect ledger.in where …         // reads window ②
```

**Three quantifiers, nothing more** :

```rastok
expect ledger.in where data.status == "settled"            // ∃ — at least one match
expect nothing codex.in [where <expr>]                     // ∅ — zero deliveries (the void word)
expect ledger.in count == 1 where data.charge_id == "ch_1" // exact count
expect "charged exactly once" ledger.in count == 1 …       // optional label, for reports
```

- **Every typed output is observable by port.** `app` remains the pre-bound
  identifier for the `: App` entry graph, so declared App boundary ports use
  `app.<port>`; node outputs use their qualified node path (`pay.exit`,
  `process.err`, …). The Lab reads the resolved `UnwiredPolicy` exactly as
  production does. There is no synthetic root-error port.
- **Expectation types are source-visible.** When an observed port has no type
  annotation available while the Lab source is checked, its predicate sees
  `data: variant`; structural access then requires explicit `is`/`as`
  narrowing. Pinning the port type gives the predicate that precise type.
  This is a check-time ergonomics rule, never a runtime fallback from an
  unresolved validated-plan port (the latter is categorically forbidden).
- **Supervision is not a port observation.** A VM fault currently fails the
  scenario; the scenario language has no positive `expect fault` form yet.
  A future supervision slice may add one explicitly, but it must not recreate
  a synthetic `err` producer or graph envelope.
- **Black-box only** : expectations target ports ; Node-local `var` state and
  runtime instance context are not directly assertable — the golden trace (§6) sees
  everything anyway, and scenarios stay robust to internal refactorings.

### 5.3 Civil time in scenarios — `origin`

Virtual time is relative ; `Calendar` (cron) needs an absolute anchor. A
scenario may declare one :

```rastok
scenario monday_report {
  origin = timestamp("2026-01-05T00:00:00Z")     // virtual t=0 ≡ this civil instant (a Monday)
  advance duration("9h")
  expect report.in where …
}
```

Absent : epoch default — and using `Calendar` in a scenario without a
declared `origin` is a **warning** (your cron fires at instants you did not
choose).

### 5.4 Fault injection

`crash after <selector>` stops the run at a **journal record boundary** — the
only well-defined interleaving points ; volatile state is lost, the journal
survives per its durability scope. `restart` runs the standard recovery
(the fold + the outbound triage, delivery-semantics §7.2). The selector anchors
on records : `<node>.<record kind>[#n]` — e.g.
`crash after reserve.outbound_open`,
`crash after route.tx#3`. Exercised by sim 4.


## 6. Golden traces — freezing behavior at the byte

The run of a scenario produces a journal (the three record types). The
**golden trace** is that journal, **canonically serialized** ; its **blake3**
is the behavioral fingerprint, versioned next to the scenario :

```
   scenario ──run──► journal ──canonical serialization──► bytes ──blake3──► fingerprint
   golden/happy_path.trace  ◄── byte compare ── identical → PASS
                                                different → the RECORD-LEVEL DIFF
                                                (first divergent record, both versions shown)
```

### 6.1 Byte-stability conditions (the honest list)

Byte-identical **iff** :

1. same graph — the canonical `.rkg` hash (the `graph_version` manifest) ;
2. same scenario text (the fixture is the world-half, by construction) ;
3. same normative schema revision (the `meta.sys` Avro schema is
   specification-owned) ;
4. same guest-language, compiler/lowering-profile and `GuestExecutor` versions
   (jq / SQL pinned in the manifest) ;
5. **the fingerprint covers the CAUSAL CORE only** —
   clock-valued fields (`committed_at`, outbound arming `t`) are observation
   annotations, journaled but outside the blake3 ; in simulation they are
   virtual (deterministic — the display may show them), in production they
   are wall-clock ; the fingerprint is identical either way ;
6. **canonical serialization AND evaluation** — field order = spec schemas;
   fixed unsigned CBOR table keys use table order; struct fields and
   state-delta paths use UTF-8 name order; `app_meta` triples use
   UTF-8 `(zone, field)` order; and Rastok `Value::Map` pairs use the
   key's canonical tagged bytes (so `"b"` precedes `"aa"`);
   the homegrown Rastok expression evaluator guarantees **sorted-key iteration inside
   macros** (`map`/`filter`/`exists` over a map) — iteration order is a
   promise *because we own the evaluator*, so a macro-produced list is
   byte-stable too.

Not in the trace : observability spans, real durations, **SNAPSHOT records**
(derived, non-causal — delivery-semantics §3) — the parallel plane and the
index leave no fingerprint.

### 6.2 The three uses

1. **The net under the expectations** : `expect` checks what you thought to
   assert (a few predicates) ; the trace checks *everything* (thousands of
   facts), at zero writing cost. A refactoring that changes behavior where no
   expectation looks → a trace diff.
2. **The determinism leak-detector** : two runs of one scenario must give the
   same bytes — or nondeterminism escaped (an unsorted map, a hidden clock,
   a violated arming order).
3. **The runtime's conformance suite** : the reference simulations' golden
   traces are derived *by hand from the spec* ; every conforming runtime must
   reproduce them byte-for-byte. This gives the specification executable
   teeth. Regulatory corollary : the trace is
   auditable evidence, and every intended behavior change is an explicit,
   reviewed **`--bless`** (regenerate + commit ; the git diff is a behavior
   diff). Never a silent regeneration.


## 7. The outcome-space checklist — enumerable, therefore checkable

In most languages "test all the outcome cases" is hope. In Rastok the closed
outcome surface of an app is **statically enumerable** :

- every effective output port is **typed** → enum/union cases and
  completion combinations form a finite declared set where the type is
  closed;
- the lifecycle events are a **closed registry** → `l:timeout`, `l:evict`,
  `l:closing`, `l:recover`… applicable per node, listable ;
- effects are **declared** → every unconfirmed compensable recovery or
  bounded-Closing cancellation has its typed `OutcomeUnknown` case (an
  `irreversible` is always keyed and resolves by recall), every `compensable`
  its failing-compensation case,
  every retry Loop its `LoopExhausted`, every saga its `rollback_failed` ;
- the expression checker already emits its **residual obligations** (bare
  `meta.app` reads, `.orValue()` branches, fallible narrows).

So the compiler can print the checklist, and the simulator can score it :

```
   rastok simulate --coverage
   23 enumerated cases · 14 covered by a scenario · 9 silent :
     hold       : OutcomeUnknown        — no scenario   (compensable, no key)
     settle     : JoinExpired           — no scenario
     pay        : LoopExhausted         — no scenario   (retry Loop)
     …
```

This is the **generation compass** for the LLM-assisted workflow (§8) — and
"failure-space coverage" is a number that means something, unlike line
coverage. The property-based layer (obligations → generated inputs,
QuickCheck-style with shrinking, replayable by determinism) is deferred.


## 8. The Gherkin bootstrap — optional, never required

The intended authoring flow (LLM-assisted, model-constrained) :

```
   ① the dev/PO writes INTENT — Gherkin, business words
        Feature: checkout        ⇒ engenders ⇒  a simulation in `lab checkout`
        Scenario: declined card  ⇒ engenders ⇒  scenario declined_card
        Given / When / Then      ⇒ fixture prelude / action steps / expectations
   ② the LLM translates into .rkg scenarios — constrained by the model :
      typed ports, declared effects, enumerated err sets, the §7 checklist
   ③ the LLM fills the checklist (happy paths AND every enumerated failure),
      interrogating the dev only where the answer is business
      (« what SHOULD happen when payment declines after the hold ? »)
   ④ the suite runs byte-deterministic ; golden traces freeze it
```

Classic BDD rots at the glue (step definitions, hand-written, silently
stale). Here the glue layer **does not exist** : the LLM emits `.rkg`
directly, the compiler checks it against the model, the simulator verifies at
the byte. Gherkin remains a *front-end convention* — documented, optional,
with `Background` ↔ a simulation-level fixture prelude, `Scenario Outline +
Examples` ↔ parameterized scenarios, `Tags` ↔
`rastok simulate --tag` — all three are deferred sugar candidates. The
`.rkg` scenario is the only executable truth.


## 9. The prod → fixture bridge

Because the fixture is the journal's world-half (§2), replaying a production
incident is a **mechanical projection**, not inference :

| Production journal slice | → | Scenario step |
|---|---|---|
| INBOUND records | → | injections (same envelopes, **same `inbound_seq`** — the injection's optional `seq` clause : `ping.out = { … } seq 105` ; hand-written scenarios omit it and use the fixture counter) |
| dated decisions (`l:*` transactions) | → | the `advance` script — **times rebased** (t₀ = first record, deltas preserved) ; the bridge sets the scenario's **`origin` = the slice's first civil instant**, so window/cron arithmetic is preserved (§5.3, execution-model §9.3) |
| OUTBOUND `closed(response)` | → | generated mock branches : the partner's real replies, **guarded by the recorded request id** (`on in where meta.sys.id == 0x9f42… { out = … }`) — ordinary mock grammar, zero new machinery |
| OUTBOUND `closed(timeout)` | → | `{ out = nothing }` (the chrono re-does its work) |
| crash + recovery (visible in the journal) | → | `crash after <record>` / `restart` |

The **slice** is well-defined : the transitive closure of a lineage through
the 3-piece provenance (parent chain + join contributions + per-hop logical
instance addresses).

This table is a *completeness contract on the journal* : every row requires
only data the three record types already carry. The extraction tooling itself
is deferred ; it requires `durability = global` and
**redaction hooks** (a prod slice is real data, PII included ; the export
anonymizes before becoming a committable scenario).

The product loop this closes : a prod incident becomes a scenario ; the fixed
scenario becomes a golden-traced locked test ; the hardening backlog feeds
itself.


## 10. Experiment — the third pillar of the Lab

An `experiment` runs the **real App** — real adapters, at least one real
Connector boundary, real time — under an **external deployment profile** :

```rastok
lab checkout {
  simulation payments on app { mock bank { … } ; scenario accepted { … } }

  experiment payment_sandbox on app {
    profile = "bank-sandbox"          // an external profile REFERENCE —
                                      // no secret, no endpoint in the DSL
    mock notifier { … }               // mocks stay legal for boundaries NOT under test
    scenario accepted { … }           // same scenario / injection / expect grammar
    scenario declined { … }
  }
}
```

Rules :

- **Same grammar, one exclusion** : injections and expectations are the
  simulation grammar (deliberate — future scenario reuse/extraction stays
  possible) ; **`advance` is a compile error** inside an experiment.
  Virtual time belongs to the simulation ; an experiment waits in *real,
  bounded* time (the bound is a profile-level knob, deliberately not new
  DSL surface).
- **An experiment, never a proof** : an experiment result is evidence
  about *this run against these partners* — it is never promoted to a
  universal claim.
- **Ingress reality — an open gap in the experiment contract** : an
  injection is *defined* as a write at a source
  Connector's **output** (§2 Boundaries) — in an experiment this
  exercises the graph but **bypasses the real inbound adapter**
  (listener, codec, auth, transport). Testing a real *inbound* boundary
  therefore requires either a **partner-side driver contract** (the
  scenario drives a real client against the real listener) or a
  **profile-specific lowering** of injections into real requests.
  Neither is specified yet ; until then an experiment's *real*
  boundaries are its outbound/callee sides, and inbound stays
  fixture-driven.
- **Same-shape journal** : an experiment produces a production-shaped
  journal, exploitable for audit, diagnosis and reproduction — which is
  exactly what feeds the bridge (§9) :

```
real experiment failure
  → journal
  → mechanical projection of its world-half (§9)
  → deterministic fixture
  → a new simulation Scenario
  → fix
  → new proof obligations, new experiment run
```

**The assurance chain, explicit** : a (future) Theorem proves the Graph
*under the Connectors' declared contracts* (stdlib.md); an Experiment
checks empirically that the real adapters and partners honor those
contracts. Neither layer replaces the other.

## 11. Theorem — declarative, reserved

`theorem <name> over <simulation> { … }` is a lab declaration, sibling of
the simulations. `over` names the simulation whose mocks and hypotheses
close the graph ; the theorem states a **universal property over all
admissible trajectories** of that (graph + simulation) pair. It is not a
scenario and is never "run a few times".

**The universal quantifier is intent, not yet semantics.** Today's mocks
are deterministic and scenarios are concrete traces — they do not define
the set « all admissible trajectories ». Before the first theorem can
mean anything, the **admissible-behavior system** must be specified :
the transition system a Simulation closes — input domains (not single
injected values), the mocks' nondeterministic choice points, admissible
sequences and interleavings, fault and timing models, bounds, and the
fairness assumptions under which liveness claims hold. Only then does
`∀ trace` quantify over something. That definition is a prerequisite of the
formalization ladder ; the property language comes after it, not before.

The body is a **reserved surface** : parsed for balance, unspecified until
the admissible-behavior system above is defined and the first mechanically
dischargeable theorem lands (the bounded-Loop example,
[loop.md](loop.md) §15). There is **no
`check` block** : `check` is imperative — an *operation*, not a
declaration. The registers are :

```
declarations : lab, simulation, experiment, scenario, mock, theorem
operations   : simulate, experiment, challenge, prove
```

(`rastok check` remains the CLI's static parse/type/validate command.)

Future results are **epistemically exact** — nothing else is admissible :

| Result | Meaning |
|---|---|
| `proved` | holds over ALL admissible trajectories |
| `proved under assumptions […]` | holds, hypotheses listed explicitly |
| `refuted` | a **minimal counterexample**, exported as a replayable Scenario |
| `no counterexample within bounds` | bounded challenge exhausted its bounds — NOT a proof |
| `unknown` | an opaque frontier (guest program, LLM, partner) precisely identified |

A bounded challenge never yields `proved` — unless the verifier establishes
that the bounds cover the entire state space. An opaque guest program (jq,
SQL) or Connector (`Process`, an LLM partner) can be : formalized ; abstracted by an
explicit contract ; treated as nondeterministic ; or left opaque with
result `unknown`. An opaque block never produces a false `proved` — but a
theorem may still be proved if it holds **for every possible output** of
the opaque block.

## 12. Non-goals of this doc (deferred, with homes)

- **Mock invocation counter / stateful mocks** — open point, admitted on a
  real case only.
- **Internal-state expectations** — black-box expectations + golden traces
  suffice for now ; revisit only with evidence.
- **`Background` / `Scenario Outline` / `Tags` sugars** — deferred
  candidates (§8).
- **Property-based generation** — deferred ; builds on §7's checklist.
- **Extraction tooling + redaction** — deferred (§9).
- **Simulating the cluster** (multi-host) — deferred until the cluster
  contract exists.


## 13. Conformance obligations

- The reference scenario must produce the complete hand-derived, ten-record
  golden trace.
- Identity vectors must cover all six ids, distinguish the four collision
  cases, and reject boundary-shift and embedded-frame ambiguity using the
  pinned framing in envelope.md §2 (domain byte + u32-LE length prefixes +
  blake3/128).
- Repeating the same scenario must produce byte-identical causal cores.


## 14. Related docs

- [execution-model.md](execution-model.md) — virtual time, the `advance`
  sweep, dated events, the node transaction.
- [delivery-semantics.md](delivery-semantics.md) — the journal (INBOUND /
  TRANSACTION / OUTBOUND), recovery, and boundary completion.
- [envelope.md](envelope.md) — the id recipe the traces fingerprint ;
  storage form (the trace serializes it).
- [grammar.md](grammar.md) — the simulation grammar productions ;
  expectations are Rastok expression predicates and the absent-field grid applies.
