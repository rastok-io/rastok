# Rastok — White Paper

> **Integrate AI faster. Keep production calm.**
>
> **Rastok is the boxes-and-arrows paradigm, rebuilt as an engineering
> discipline.** The flows that organizations already draw —
> integrations, sessions, agents — are written in a dedicated, typed
> domain-specific language (DSL), versioned in Git, exercised through
> deterministic simulation, and replayed from a causal journal. The same
> source is designed to move through a standard GitOps delivery process,
> from one native runtime process to a sharded deployment.

Rastok is an **open-source runtime and typed graph DSL for stateful,
effectful integrations**. A graph connects existing services, models,
tools, data, and business processes through three kinds of nodes:

```text
External world
      │ read
      ▼
┌──────────────────────────┐
│ Connector                │
│ explicit effect boundary │
└────────────┬─────────────┘
             │ Envelope
             │ meta + typed data
             ▼
┌──────────────────────────┐
│ Operator                 │
│ pure graph logic         │
└────────────┬─────────────┘
             │ Envelope
             │ meta + typed data
             ▼
┌──────────────────────────┐
│ Connector                │
│ explicit effect boundary │
└────────────┬─────────────┘
             │ write
             ▼
       External world
```

An **Envelope** is the immutable message carried by an edge. An
**Operator** transforms envelopes and may update graph state, but has no
external side effects. A **Connector** is the explicit boundary where
the graph reads or changes the external world. A **Graph** composes
these nodes. In the Lab, controlled handlers replace the Connectors
while the same Operators and the same checked graph continue to run.

AI agents are a useful stress test: they combine long-lived state,
external effects, variable cost, and probabilistic results in one
operational flow. The same controls also apply to less probabilistic
integrations — and when a particular model changes, the surrounding
flow still remains.

The economics in one line: **programming languages let you build a
workflow — Rastok lets you operate hundreds of them.**

This document is the white paper and statement of intent of Rastok. It
is the first file a new contributor, adopter, investor, or
formal-methods reviewer should read to understand what Rastok is trying
to make possible.

Read it as the book of the project: why Rastok exists and what it
provides (§1), what a graph is (§2–§5), how the Lab makes it testable
and eventually verifiable (§6), and where the normative specifications
take over. The DSL grammar, runtime contracts, examples, architecture,
and roadmap refine the principles stated here.

This document is comprehensive on purpose. Tactical guides live
elsewhere; this is the conceptual constitution. Runtime internals are
documented separately in [ARCHITECTURE.md](ARCHITECTURE.md), next to
the implementation they describe. Comparisons with adjacent systems
live in the annex (§9), after the model they compare.

And for the reader who wants the model up front, in one formal
sentence:

> *Rastok is a strictly acyclic graph of typed Nodes whose logical instances
> communicate through immutable envelopes, commit Node-local state and
> effects through a causal journal, and are specifiable by executable
> simulations.*

Every word of that sentence is unpacked in §2–§6.

## About the name

Rastok is named after **Rastoke**, the watermill village of Slunj,
Croatia, built where the Slunjčica river branches into cascades before
joining the Korana. The Croatian name comes from *ras-* + *tok* —
literally "the flow, branching apart." For centuries the village has
channeled that branching water through its mills: every stream visible
from the footbridges, every drop doing work on its way through, the
whole thing legible at a glance and still running. A village-sized
dataflow graph, powered by water — the name is a homage, and a design
target.

---

## 1. Why Rastok

### 1.1 Problems Rastok is designed to solve

Modern interactive systems — especially AI agents — combine several
difficult operational properties:

| Problem | What commonly goes wrong | Rastok's contract |
|---|---|---|
| Long-lived keyed state | Session state is scattered across workers, caches, and correlation tables. | A Node selects a logical instance by key; its private state and mailbox serialize work, while lifetime remains an independent policy. |
| Events arriving out of order | Each flow grows its own correlation state machine and locking rules. | Bounded joins correlate typed envelopes by key. |
| External side effects | HTTP calls, processes, files, payments, and retries are hidden inside application code. | Every external interaction crosses a declared Connector. |
| Probabilistic components | An LLM may return different answers for the same request, with variable latency and cost. | The journal records each boundary outcome; the graph isolates and measures the probabilistic surface. |
| Time-dependent behavior | Code reads `now()` or sleeps, so tests wait in real time and cannot reproduce the same timeout race. | Time enters through a Connector and becomes controlled virtual time in the Lab. |
| Recovery and audit | Effects and state updates leave separate, incomplete records. | Transactions and boundary outcomes form one causal journal. |
| Operational policy | Every team reimplements retries, timeouts, supervision, backpressure, and cost metering. | The graph declares shared runtime policy once. |
| Change control | Visual canvases and custom glue are difficult to review, compare, and generate safely. | Typed text is versioned in Git, checked before execution, and projected into a diagram. |

Three questions summarize the intended result:

1. *Can you replay yesterday's incident along the same causal path?*
2. *Can you test the whole flow without calling the real LLM or bank?*
3. *Can you show which input caused each external effect?*

Time matters in all three. Reading the wall clock is an external
observation, just like reading an API response. If a timeout depends on
an invisible `now()`, the value appears in neither the fixture nor the
journal. A test must wait, and a replay may take a different branch.
Rastok therefore treats time as an explicit effect (§5).

### 1.2 Why use a DSL instead of a general-purpose language?

Every example in this document can be written in Python, Java, Rust,
or Node.js. An experienced developer — or an LLM — can produce a
working version of the approval flow in §1.5 in an afternoon: a map,
two HTTP handlers, and a timer.

That is not the expensive part. **Writing one workflow is cheap.
Operating hundreds of them for years is not.** Without a shared
runtime, each team must design, build, test, and maintain its own
keyed state, correlation, recovery, simulation, journal, supervision,
backpressure, metering, and diagnostics.

The benefit is organizational as well as technical: one set of
conventions instead of one per team; incidents that are replayed
instead of argued about; and less bespoke platform engineering around
every new flow.

> Rastok does not replace a general-purpose programming language.
> It provides the common platform that organizations otherwise build
> around their flows. Programming languages let you build a workflow —
> **Rastok lets you operate hundreds of them.**

#### 1.2.1 The test environment is source code

One item above deserves to be pulled out, because it changes daily
practice. In a flow platform or custom codebase, exercising a complete
integration against its real boundaries often means having a database,
the partner's API, a broker, credentials, test data, a staging
environment, and an available partner team. Testing difficult
cases — events out of order, a 48-hour timeout, a crash right after a
payment, a reply that arrives too late — ranges from laborious to
practically impossible.

In Rastok, the world around the graph is part of the simulation
specification: partners, processes, LLMs, databases, *and the clock*
are declared — so the complete flow builds and runs on a laptop,
whole-world simulated, **before the real systems are available**, and
each difficult case is a few scenario lines (§1.5). This is not unit
mocking: the simulation
closes the *entire* world, controls time, arrival races and faults,
and produces a deterministic, replayable trace.

> In traditional integration tools, the test environment is
> infrastructure. In Rastok, it is source code.

#### 1.2.2 Why organizations already buy integration platforms

This value proposition is established. Organizations did not adopt
NiFi, Talend, or BizTalk because boxes and arrows were impossible to
program in Java. They adopted those platforms because *developing,
deploying, supervising
and maintaining hundreds of bespoke integrations* was expensive and
risky — the demand served by workflow platforms such as Temporal. Rastok is that
proposition, modernized: a textual source of truth instead of a
canvas, types, Git and code review, deterministic whole-world
simulation, a journal built for replay, LLM-friendly generation
(§1.4) — with the visual graph still there, derived from the source
that actually runs.

The general-purpose language keeps its role: the *inside* of a
Connector — native code against the real world — is Rust; your
services stay whatever they are. Rastok occupies the layer between
them: the flow, its state, its time, and its evidence.

### 1.3 The Rastok answer

The core move is an inversion. In many visual-flow tools, the diagram
*is* the program — and even when its snapshots are versioned, a canvas
snapshot is neither an intentional, typed source a reviewer can read
nor a world that can be closed for deterministic testing. In Rastok,
**the program is text, and the diagram is a projection** — derived
from the source, always accurate, and never a second artifact to keep
in sync. Text is what Git versions, reviewers compare, CI checks, type
systems verify, and LLMs generate fluently.

> Rastok = a typed DAG of Node-owned logical instances,
> communicating through immutable envelopes, with every effect
> crossing a declared boundary.

The platform of §1.2 follows from four decisions:

1. **The topology is a static, acyclic graph** (§2.6). Every path a
   message can take is visible in the source. What cannot be enumerated
   cannot be tested, replayed, or proved.
2. **All contact with the outside world crosses a declared Connector**
   (§2.7) — HTTP, processes, databases, *and the clock itself* (§5).
   Mock the Connectors and the graph becomes a closed, deterministic
   system. That makes simulation (§6) a first-class artifact
   instead of a best-effort test harness.
3. **A Node may select its own logical instance by key** (§3), independently
   of its lifetime. Its `var` state remains private to that Node. Processing
   is serialized within one instance and may run in parallel across addresses;
   every keyed owner is also a future sharding boundary (§4).
4. **Every transaction is journaled.** A node consumes an envelope,
   updates its state, and emits atomically. The journal records the
   outcome of every arrival race. A run is a pure function of
   `(graph, journal)`: *journal-relative determinism*, the replay
   contract behind questions 1 and 3.

More software now delegates decisions to LLMs. Traditional code is
expected to be deterministic; an LLM response is sampled from a
probability distribution. A production system therefore has to compose
deterministic and probabilistic components without allowing uncertainty
to spread through the whole design.

The lifecycle Rastok supports is **route → meter → harden → prove**.
First, route both kinds of components through one explicit graph.
Second, meter every boundary crossing in the journal. Third, harden
behavior that has become stable. Finally, prove selected properties of
the deterministic fragment when the necessary verification support
exists.

For example, begin with an LLM Connector that classifies support
requests. The journal records its answers, latency, and cost. If refund
requests become a stable rule, move that case into a typed Router and
keep the LLM only as a fallback for ambiguous requests. Replay the Lab
scenarios and compare their traces before deploying the change. Each
iteration shrinks the probabilistic surface — and the token bill —
without a rewrite. Simulations check declared behavior; proving stronger
properties belongs to the future `theorem` level (§6.1).

### 1.4 The DSL in the LLM era — the constraint is the accelerator

Ten years ago, a DSL's weakness was the cost of writing it: one more
syntax to learn, no IDE muscle memory, a thin labor market. LLMs invert
that economics. Structured, constrained text is exactly what a model
generates best — and a general-purpose language is exactly what it
verifies worst.

Ask an LLM for the approval flow in Node.js and it will happily
produce a hundred plausible implementations: different state models,
different retry policies, different error conventions, different tests
and instrumentation, architectures that resist comparison. All of them
can work. None of them look alike — and each demands line-by-line
review, then its own five years of operation. Ask for a Rastok graph
and the generation is fenced by a grammar, a type system, a static
topology, one state model, a hard separation between pure logic and
effects, and one runtime. The result is homogeneous, reviewable
on one screen, immediately simulable against the declared scenarios,
immediately journaled, immediately visible as a diagram. A malformed
graph is rejected by the compiler; a wrong one by the simulation —
before anything touches the world.

The same afternoon produces, in one case, *a* solution; in the other,
an artifact the whole ecosystem — simulator, journal, dashboards,
auditors, the next developer, the next LLM — exploits without reading
it twice. Generation is cheap either way. **Operation** is where the
paths diverge:

> An LLM makes code cheap to write.
> Rastok makes flows cheap to operate, audit and evolve.

This is also what makes the graph-generation workflow of §6.5
credible: the LLM proposes, and the simulations constrain and check
the proposal. A constrained language is not a cage; it is what turns
machine-written flows into governable ones: **an LLM can generate
another workflow — Rastok
makes every generated workflow look, behave and operate the same.**
The loop does not stop at generating a happy path. Each counterexample,
production incident, or failed experiment becomes another scenario
that constrains the next candidate. This accumulating executable corpus
is one of the project's central advantages.

### 1.5 A first taste — an asynchronous approval

The next two sections make §1.2 concrete. Read them with the right
question in mind — not « could I code this? » (you could), but
« what would I otherwise have to build, test and operate around it? »

The flagship example is deliberately *not* an HTTP-in, HTTP-out
transformation — that is the happy path every flow tool demonstrates
well. It is the case where they all get uncomfortable: **two
independent events that must be correlated, arriving in any order, with
a business deadline**. An expense request arrives on one channel; the
manager's decision arrives on another. Independent queues, retries, and
service latency mean that the decision can reach Rastok first even if
the human made it after seeing the request. An approval may reach the
accounting system; a rejection is a separate business outcome. If one
side never arrives, the bounded join reports the technical
`JoinExpired` error instead of leaving a stuck message.

```rastok
graph expense_approval : App {                     // `: App` marks the deployment entry
  output rejection

  connector requests   : HttpServer { listen = "0.0.0.0:8080" ; path = "/requests" ; codec = json }
  connector decisions  : HttpServer { listen = "0.0.0.0:8080" ; path = "/decisions" ; codec = json }
  connector accounting : HttpClient {
    url             = "https://accounting.internal/entries"
    method          = "POST"
    codec           = json
    timeout         = duration("10s")
    effect          = irreversible
    behavior        = deterministic
    idempotency_key = meta.ext.request_id
  }

  operator decide {                                // one logical instance per expense
    instances = { key = meta.ext.request_id }
    lifetime = { kind = idle ; after = duration("72h") }

    output approval
    output rejection

    on (request, decision)
      where decision.data.approved
      within duration("48h") {
      approval = {
        meta.ext.request_id = request.meta.ext.request_id   // a join picks explicitly; common fields merge (CommonOrAbsent)
        data = { request_id = instance.key,
                        amount     = request.data.amount }
      }
    }

    on (request, decision)
      where !decision.data.approved
      within duration("48h") {
      rejection = {
        meta.ext.request_id = request.meta.ext.request_id
        data = { request_id = instance.key,
                        amount     = request.data.amount,
                        reason     = decision.data.reason }
      }
    }
  }

  requests  -> decide.request                      // wiring is explicit and graph-level
  decisions -> decide.decision
  decide.approval  -> accounting
  decide.rejection -> rejection
  accounting.out -> nothing                        // reply discarded ON PURPOSE (explicit, or it won't compile)
  // `decide.expired` carries JoinExpired. Its contract journals an unwired
  // value; wiring or `-> nothing` can override that explicit policy.
}

lab expense_approval {
  simulation approvals on app {
    scenario decision_before_request "the decision arrives first; correlation still works" {
      mock accounting {                              // mock the CALLED system…
        on in { out = { entry_id = "acc-1" } }
      }
      // …but DRIVE the sources by injection (they are inputs, not partners) :
      decisions.out = { meta.ext.request_id = "r-42",
                                 data = { approved = true } }
      requests.out  = { meta.ext.request_id = "r-42",
                                 data = { amount = 1250 } }

      expect "posted to accounting" accounting.in where
        data.request_id == "r-42"
    }

    scenario rejected "a rejection is a business outcome, not an accounting entry" {
      decisions.out = { meta.ext.request_id = "r-43",
                                 data = { approved = false,
                                                 reason = "policy" } }
      requests.out  = { meta.ext.request_id = "r-43",
                                 data = { amount = 1800 } }

      expect "rejection is explicit" app.rejection where
        data.request_id == "r-43" && data.reason == "policy"
      expect nothing accounting.in
    }

    scenario approval_timeout "no decision within 48 hours; the join expires" {
      mock accounting { on in { out = { entry_id = "acc-1" } } }

      requests.out = { meta.ext.request_id = "r-7",
                                data = { amount = 990 } }
      expect nothing accounting.in                   // window ① : nothing moves on arrival

      advance duration("48h")                                    // virtual time — nothing sleeps

      expect "expiry is typed" decide.expired where data is JoinExpired
      expect nothing accounting.in                   // window ② : and nothing moved at expiry
      // (each action step — injection or advance — opens a fresh observation
      //  window ; only asserting the absence in BOTH windows covers the whole
      //  run : the money never moved)
    }
  }
}
```

The message of this example: **two independent events may arrive in any
order; Rastok correlates them by key, separates approval from rejection,
and bounds the wait.** Note what is already visible:

- **Stateful correlation is the model, not plumbing.** `instances.key`
  selects the Operator instance; the join `on (request, decision)` matches
  the two legs of the *same* expense automatically, whatever the arrival
  order.
- **Business outcomes and technical outcomes remain distinct.** The two
  guarded handlers emit `approval` or `rejection`. Missing input is not a
  business refusal: after 48 hours it becomes the typed `JoinExpired`
  case on `decide.expired`.
- **The simulation drives the world.** The *called* system
  (`accounting`) is mocked; the *sources* are driven by injection;
  `advance duration("48h")` moves virtual time deterministically. Nothing here
  touches a network or a clock.
- **The lab is the App's twin.** `lab expense_approval` binds to the
  homonymous `: App` graph (§2.2, §6) and is never deployed; its
  simulation names the graph it closes (`on app` — `app` is pre-bound),
  and node references resolve in the unit's
  namespace as root-relative qualified names.

Now count what this file does **not** contain: no state-machine class
for out-of-order arrival, no correlation table and its locks, no
persistence layer for the pending request, no scheduler for the
48-hour expiry, no mocking framework, no fake clock, no audit
middleware, no recovery logic, no replay tooling. The business logic —
one join, one bound, one emission — is the easy part, *and it is the
whole file*. Everything a platform team would normally build around it
is the runtime's contract, not your code.

And the same scenario grammar keeps going where a hand-rolled harness
stops: `crash after <record>` / `restart` kills the process at a chosen
journal boundary and exercises recovery (reference simulation 4 replays
a saga through a crash); re-delivering a duplicate is just injecting
the same envelope twice — whether the flow deduplicates or double-fires
becomes an *asserted* behavior, not a production surprise;
`expect … count == 1` asserts that the scenario produces exactly one
effect; a partner that
answers *too late* is one dated line in a mock (`after
duration("12s")`, §6.2); and the run's evidence splits cleanly: the
journal provides the proof for replay and audit, while the golden trace
makes the resulting behavior regression-testable byte-for-byte.

### 1.6 A second taste — supervising agent sessions

The same model on the agentic terrain: a gateway that gives every
conversation its own supervised OS process (here `codex`, an AI coding
agent), with bounded concurrency and per-session lifetime. Abridged from
[docs/examples/teams-codex-gateway.rkg](docs/examples/teams-codex-gateway.rkg):

```rastok
graph sessions {                       // declaration = instantiation
  instances = { key = meta.ext.session_key }
  lifetime  = { kind = idle ; after = duration("2h") }
  val cwd = "/workspace/" + instance.key           // immutable instance context

  connector codex : Process {
    command = "codex"
    args    = ["app-server"]
    mode    = service                              // resident child process, one per session
    cwd     = cwd
    codec   = jsonl
    effect  = idempotent
    behavior = probabilistic
    timeout = duration("120s")                                 // per-call budget
    restart = { backoff = "1s..30s", max_attempts = 5 }
  }

  in        -> codex.in { capacity = 1024, backpressure = block }
  codex.out -> out
  codex.err           -> nothing      // abridged: production wires a diagnostic sink
  codex.exit          -> nothing      // terminal outcome of each correlated call
  // err is stderr; exit carries Exited/TimeoutExpired/etc. Every
  // boundary crossing is journaled, so
  // per-session cost is an aggregate over the journal, not a guess.
}

graph agent_gateway : App {
  connector teams_in  : HttpServer {
    listen = "0.0.0.0:8080" ; path = "/api/messages" ; codec = json
  }
  connector teams_out : HttpClient {
    url             = "https://smba.trafficmanager.net/"
    method          = "POST"
    codec           = json
    timeout         = duration("10s")
    effect          = irreversible
    behavior        = deterministic
    idempotency_key = meta.sys.id
  }

  teams_in -> sessions -> teams_out.in  // a Graph is a Node: wired by name
  teams_out.out -> nothing              // delivery receipt discarded on purpose
}

lab agent_gateway {
  simulation conversations on app {
    scenario first_turn "a user message reaches codex ; the reply reaches the user" {
      mock sessions.codex {             // a qualified path into the keyed sub-graph
        on in where data.type == "message" {
          out = { method = "item/agentMessage",
                  params = { message = "Bonjour" } }
        }
      }
      mock teams_out {                  // EVERY called external system must be covered —
        on in { out = { delivered = true } }   // an uncovered call fails the scenario loudly
      }

      teams_in.out = { meta.ext.session_key = "conv-1",
                                data = { type = "message", text = "Bonjour" } }

      expect "reply reaches the user" teams_out.in where
        data.params.message == "Bonjour"
    }
  }
}
```

The rule this example makes visible: **declaring is placing, key selection is
cardinality, wiring is naming.** A `graph` is a Node like any other —
`sessions` is declared once and wired by name. Because it has
`instances.key`, the runtime materializes one graph instance for each
distinct session key. Every child Node belongs to that graph instance.
A child with its own `instances.key` adds an independently shardable address
frame; a child without one has one singleton instance inside its parent Graph
instance. Live-cardinality caps and placement remain deployment policy rather
than source semantics.

Beyond the mechanics, note the operational posture: a thousand
concurrent sessions are one declaration and one knob — and one journal.
Per-session cost (every LLM and API crossing is an OUTBOUND record) is
an aggregation over the journal, not an instrumentation project; a
dying `codex` is a typed terminal error with a declared restart policy,
not a 3 a.m. mystery; and the whole gateway is simulated — process,
clock and Teams included — before it ever touches production.

The HTTP Teams edge is not essential to the shape. The M1.6 design reference
[codex-websocket-gateway.rkg](docs/examples/codex-websocket-gateway.rkg)
replaces it with `WebSocketServer`: each accepted connection supplies one
logical session reference, complete text/JSON messages cross ordinary
`in`/`out` ports, and one `exit` reports the connection's termination. The
same graph works whether the Driver negotiated HTTP/1.1 Upgrade, HTTP/2
Extended CONNECT or HTTP/3 Extended CONNECT.

### 1.7 When Rastok is more platform than you need

Honesty about scope buys trust, so let us draw the lower bound: a
fifty-line Python script does not need a dataflow runtime. A single
Lambda gluing two APIs, a plain CRUD service in front of a database, a
one-off migration, a notebook — none of these justify a DSL, and a
general-purpose language plus an LLM serves them perfectly. Rastok
below its threshold is ceremony.

The threshold is crossed when the *surroundings* accumulate: several
workflows sharing state and teams; effects that must be compensated
and audited; LLM calls whose cost and behavior must be metered;
long-lived sessions with processes attached; incidents that must be
replayed rather than guessed at; compliance questions that name data
paths. Each of those alone is a library. Together they are a platform —
and that is the moment the afternoon prototype begins its second,
invisible life as infrastructure. Adopt Rastok at the point where you
would otherwise start building the platform of §1.2.

### 1.8 Where this comes from — recurring graph traditions

Rastok's ancestry is the history of drawing systems as boxes and
arrows. These waves overlap; this is not a sequence in which one
approach replaced the previous one:

```
2000s+  SOA and ESB           flows in integration platforms and buses
                             (BizTalk, Talend, NiFi)
2000s+  distributed data     flows in pipelines
                             (Hadoop, Spark, Kafka, Flink)
2010s+  microservices        flows spread across deployed services
2010s+  workflow platforms   flows return as explicit DAGs
                             (Airflow, Kubeflow, Dagster)
2010s+  low-code             flows reach a wider authoring audience
                             (Zapier, Make, n8n)
2020s+  agentic systems      flows include probabilistic calls, tools,
                             and human approvals
```

Two constants run through the story. First, **the drawing keeps coming
back** — whenever behavior is dominated by how data moves, engineers
and business people alike reach for nodes and edges. The demand is
structural: the graph is the one artifact business, ops, dev, auditors
— and now LLMs — read together; the 3 a.m. questions ("where is the
message stuck?", "what is upstream of this error?") are questions
about the drawing; regulators literally ask for it ("show me how this
data moves and who touched it"); and a newcomer in front of a map
knows where they are within a day. Second, **the era that suppressed
the drawing paid for it**: microservices scattered the flow across
dozens of repositories, and the industry spent the next decade
rebuilding sight of it — distributed tracing and OpenTelemetry are the
graph, reconstructed after the fact, at runtime, from fragments.

The tools that took the drawing seriously hit a shared ceiling from
two sides. **NiFi** (the enterprise side) got provenance, visible
queues, and backpressure right; but the canvas is the program. Its versioning
stores *snapshots of the canvas*, not a source a human writes and a
reviewer reads; there is no deterministic simulation of the whole
external world, so changes are validated in staging against live
dependencies; state and correlation are assembled from attributes and
controller services rather than keyed into the model; and reaching the
edge took a separate sub-project (MiNiFi). **The low-code wave** (the
mainstream side — Zapier, Make, n8n) proved the demand at massive scale
and matured its engines — execution history, step re-runs, pinned test
data, Git sync in enterprise tiers — without changing the model:
duck-typed steps with no contracts, exercised against real or recorded
services, governed one workflow at a time; the pattern operations
teams call *click-ops*. Organizations that outgrow either side do the
only thing left: they rewrite the flows in code, regain engineering,
and lose the drawing. The market, as it stands, forces a choice:

```
                 ENGINEERABLE
            (git, typed, tested, CI/CD)
                      ▲
                      │
     custom code      │      ★ RASTOK
     (Kafka Streams,  │      typed, text-first graphs
      Temporal,       │
      microservices)  │
   ───────────────────┼─────────────────────▶  VISIBLE
                      │       (the flow can be seen : nodes & edges,
                      │        ops and business read the same object)
                      │
                      │      NiFi · n8n · Make · Zapier · SmythOS
                      │      (the diagram IS the program,
                      ▼       stored as XML/JSON behind a UI)
                  CLICK-OPS
```

Rastok exists to make that choice obsolete. The tool-by-tool scorecard —
including what NiFi still does better today — is in the annex, §9.10.

### 1.9 Who this document is for

- **Backend developers and SREs** writing the actual graphs and
  connectors.
- **Product managers and product owners** writing simulations as
  specifications.
- **Formal-methods reviewers** assessing the model and the invariants
  it aims to make checkable — approvals, delivery guarantees, absence
  of deadlock (the future challenge and `theorem` levels, §6.1).
- **LLM-assisted developers** generating graphs from simulations.

Rastok deliberately synthesizes three traditions that have historically
lived apart — deterministic dataflow (Lustre/SCADE, Kahn networks),
supervised actors (Erlang/OTP, Orleans), and modern keyed stream
processing (Flink, Kafka Streams) — for the agentic era: long-running
conversational sessions with LLMs, process-backed agents, multi-tenant
SaaS. The full genealogy — what was taken from each tradition, what was
left behind — and the practical positioning against Airflow, LangGraph,
NiFi and friends live in the annex (§9).

---

## 2. The Core Model

This section builds the model from the inside out: the message
(§2.1), the node that processes it (§2.2), the handler that fires the
processing (§2.3), the state it may keep (§2.4), the expression and guest
languages a handler computes with (§2.5), the topology that connects nodes
(§2.6), and the place where the real world's loops are absorbed (§2.7).

A Rastok App is a graph that compiles into a target-neutral
`ExecutionPlan`. The plan is an immutable description of a state
machine. The Rastok runtime interprets that state machine in its virtual
machine.

The first runtime is native: it is written in Rust and compiled for the
underlying operating system and hardware. "Virtual machine" describes
the interpreter for the `ExecutionPlan`; it does not mean that the
runtime itself is interpreted or requires a separate managed runtime.

The plan boundary leaves room for future execution forms:

- a self-contained native bundle that packages a plan with the runtime;
- the same VM compiled for a browser or edge environment;
- a Wasm artifact for constrained deployment targets;
- ahead-of-time lowering for a proven need.

These are architectural directions, not current product commitments.

Although cycle checking may expand composition into one spatial DAG, the
validated plan retains the parent-linked Graph/Operator/Connector owner tree.
That tree is semantic: it locates Graph boundaries, instance frames, private
state and lifecycle. A compiler may flatten passive routing, but it may not
erase an owner and later attempt to reconstruct containment from a path.

Rastok recognizes exactly **three families of node**, sharing the same
underlying structure:

```
Node
├── Operator   — pure, event-driven computation inside the graph
├── Connector  — reads from or writes to the external environment
└── Graph      — composite Node, wired by name like any other Node
```

An Operator reacts to envelope arrivals on its inputs, optionally reads
and updates Node-local state in its current logical instance, and emits zero or more
envelopes on its outputs. A Connector is a Node with extra
**boundary attributes** (URL, path, command, codec, schedule, …) that
describe how it interacts with the external world (HTTP, processes,
files, databases, clocks, brokers). A Graph is a composite Node: its
body contains Nodes and edges, and its declaration is its placement.

The three families share the **same Node grammar**. Connectors add
boundary attributes — addressing the external resource and binding it
to a wire format — on top of what every Node has. Graphs add a nested
topology.

### 2.1 The Envelope, its data, and the type system

Every message flowing through the graph is an **envelope** = `meta + data`:

`meta` is the lightweight routing and provenance surface; `data` is the
single carried payload. The postal metaphor fits: `meta` is what's written
*on* the envelope, `data` is what's *inside*.

`meta` has exactly three zones. `meta.sys` is the runtime's read-only spine —
identity, lineage, role; five fields, minted fresh at every hop.
`meta.app` is the application's writable zone — its fields inferred
from the graph's own writes, with types unified across writers — and
`meta.ext` is the external-boundary context, written by connectors
(their contract sheets declare its fields) and rewritable by
handlers. There is no bare form: `meta.x` resolves to nothing.
`meta` leaves are typed scalars (or scalar arrays) — composites belong in
`data`. The normative field list, read-only rules, id derivation, storage
form and birth stamps live in
[docs/spec/envelope.md](docs/spec/envelope.md).

```rastok
out = {
  meta.app.request_city = data.city              // a scalar annotation, rides on the envelope
  data = { city = "Paris", country = "FR" }  // a typed record payload
}
```

Here, `struct` means a statically typed record. It resembles a JSON
object, but its fields and their types are checked at build time. Other
payload types include scalars, arrays, maps, enums, unions, and the
explicitly dynamic `variant` type described below.

Because emitting a structured payload is the overwhelmingly common
case, a braced value is classified once by its top-level roots
(D-2026-08-09-1). With no reserved root it is a *data record* — a fresh
typed struct payload, the input's heritable metadata kept:

```rastok
out = { city = "Paris", country = "FR" }
```

With only the reserved roots `data` and `meta` it is the
*envelope form* — the builder shown above, patching or rebuilding the
envelope itself. Mixing the two refuses at compile time
(`E-BRACE-MIXED`); business fields named like a reserved root take the
explicit escape `data = { … }`. One form, one classification.

**Static by default.** The data has a *static type*, known at compile
time. This is the Rust discipline: a statically-typed value inherits
the operator algebra of its type — string ops on `text`, arithmetic
on numbers, relational ops on tables — instead of reinventing them.
The single dynamic type is **`variant`**: a self-describing
semi-structured value (the JSON / Snowflake-Delta `VARIANT` model).
It is *safe*, `unknown`-style — it must be **narrowed** before any
structural access, and it never disables checking elsewhere.

The type system is the common core of JSON, Avro, Rust/serde, Delta,
Arrow/Polars and SQL-style tables. Supported JSON Schema, Avro and Arrow
schemas project losslessly into the same algebra, including nullable,
structural and tagged unions; an unsupported or ambiguous projection
refuses instead of degrading to `bytes` or `variant`. It keeps logical
type, internal representation, boundary codec and external content type
distinct. Persisted and transported payload representations are bytes,
but bytes are the logical type only for opaque binary data; an Avro or
Arrow buffer may represent a logical struct, collection or union. Pure
routing therefore does not pay parse cost. The exact authority is
[docs/spec/types.md](docs/spec/types.md), with host syntax in
[docs/spec/grammar.md](docs/spec/grammar.md).

### 2.2 The shared Node shape

Every Node, regardless of family, shares one declarative skeleton:

```
<family> <name> [: <annotation>] { <body> }

family     : operator | connector | graph        (a graph instantiates by DECLARATION
                                                   and wires by name ; the `: App` role
                                                   marks the deployment entry graph ;
                                                   mock, simulation-only, mirrors the
                                                   same skeleton as a stand-in)
annotation : trait/role + aspects — optional on operator/graph (implicit canonical
             types `Operator` / `Graph`), MANDATORY on connector (a boundary is explicit)
```

The port-declaration keywords are **`input`** and **`output`**; the names
**`in`**, **`out`** and **`err`** are conventional, non-reserved spellings
typed by the same mechanism as every other port. They acquire plan rows only
when declared, handled, emitted or wired; an Operator has no mandatory
`err`. A
validated plan carries an effective type for all ports, whether inferred
or explicitly pinned. For `Process`, the mapping is exact: `in` is stdin,
`out` is stdout decoded by the selected codec, `err` is stderr bytes, and
the additional `exit` output carries the tagged `ProcessExit`
(`Exited`, `SpawnFailed`, `TimeoutExpired`, `DecodeFailed`, …). A
non-zero exit is an `Exited` fact; graph policy decides what it means.

```rastok
operator <name> [: <trait + aspects>] {
  input  <port> [: <type>]   // source annotation optional; effective type mandatory after validation
  output <port> [: <type>]   // a single-in/out node just uses the defaults

  var <field>: <type> [= <default>]    // optional state fields

  instances = { key = <expr> }         // optional; absence = singleton
  lifetime  = { kind = <profile> ... } // optional; absence = persistent

  on start            { ... }          // logical instance starts lazily
  on <trigger> [where …] [having …] [within …] [last …] { ... }   // see §2.3
  on stop             { ... }          // logical instance has finished Closing
}
```

A Connector adds boundary attributes describing its bridge to the
outside world; built-ins do not re-declare their ports (their type
defines them):

```
connector <name> : <NativeType> {
  <attr> = <value>           // e.g., url = "...", codec = json ; Connectors declare NO ports
  on <trigger> { ... }       // the contract + codec resolve every typed port
}
```

A sub-graph is a Node too: **its declaration is its instance owner**. It is
wired by name and may select keyed instances exactly like any other Node:

```
graph <name> [: App | <aspects>] {
  instances = { key = <expr> }         // optional; one Graph instance per key
  lifetime  = { kind = idle ; after = duration("2h") }
  val <field> = "..." + instance.key + "..."      // immutable instance context
  ...                                  // operators / connectors / sub-graphs / wiring
}
```

Three categories of section:

- **Declarative interface** (`input`, `output`, `var`, `instances`,
  `lifetime`). **Ports
  are implicit**: `on <name>` declares an input, an assignment
  `<name> = …` declares an output, and `in`/`out`/`err` have conventional
  directions — so a Node usually declares no ports at all. A conventional
  port not used by the program does not exist in its effective table. `input` /
  `output` are written only to *pin* a type/schema, or to document a
  node with no handler (a saga) or a graph's public interface. `var`
  holds state owned only by that Node; `instances.key` selects its logical
  address and `lifetime` independently controls temporal closure.
- **Active behavior** — the `on <trigger>` handlers (§2.3). Outputs are
  assigned explicitly; **atomic multi-emit** uses tuple assignment
  `(out, exit) = (e1, e2)`. A handler may emit nothing (pure state
  update). There is no implicit yield.
- **Connector boundary** — only on Connectors: flat attributes such
  as URL/path/command, **codec** (the bytes↔value boundary cast,
  §2.1), protocol, timeout, auth and retry policy.

### 2.3 Event-driven semantics — handlers as transaction guards

A Node is an event-driven actor — *not* an FP function. A transaction is

```
transaction = (trigger, state_in) → (state_out, emissions)
```

atomic (state delta and emissions commit together), and may emit zero
or N envelopes on any combination of outputs.

A handler is `on <trigger> [where …] [having …] [within …] [last …]`,
plus the two lifecycle hooks `on start` / `on stop`. The trigger says
*which* ports fire it; the optional clauses refine the transaction
condition and bound the join state.

**Triggers.**

| Trigger        | Fires when                                                          |
|----------------|---------------------------------------------------------------------|
| `on a`         | an envelope arrives on port `a`                                     |
| `on (a, b, …)` | envelopes are present on all named ports (a **join**), zipped by arrival order within the owning Operator instance |

OR across ports is expressed as **separate handlers** (`on a { }` /
`on b { }`) — there is no `||` in a trigger.

**Refinements.** `where <pred>` is a predicate over the matched
envelopes' content; `having count(p) >= k` is the reserved quorum (k-of-n)
form. M1.4 R6 executes `where`, `within`, and `last`; `having` remains a named
future gate rather than an inert accepted clause.

**Join key = owner instance.** Every envelope reaches one logical instance of
the Operator before it can enter the join buffer, so `on (a, b)` correlates in
that address for free. There is no second `by` key authority. If a flow needs
a different correlation domain, the Operator declares the corresponding
`instances.key`. The §1.5 approval graph is exactly this: both legs select the
same request-id instance.

**Joins are bounded — and the bound is behavior.** A presence join
(`on (a, b)`) zips by arrival order within the instance and is
bounded by edge backpressure. A join whose `where` *correlates the
content* of several ports is a θ-join and must declare an explicit
bound: `last N` (count — the static memory guarantee, clock-free)
and/or `within D` (staleness — ksqlDB's `WITHIN`). Candidate tuples retain
the handler's declared port order and use per-port FIFO choices; a false
predicate consumes nothing. A candidate that
leaves the join unmatched emits **`JoinExpired`** on a declared typed
output (exactly `expired` in the M1.4 surface) — routed and observable like any other fact, never a silent
drop, never a background timer. The exact tier classification, bound
composition and lazy-eviction rules are normative in
[execution-model.md](docs/spec/execution-model.md) §7 and
[grammar.md](docs/spec/grammar.md).

Each join input is a named complete envelope (`a.data`, `a.meta`) with its own
lazy payload witness. Writable metadata merges by CommonOrAbsent; a required
downstream read that could observe absence is rejected until an explicit
write chooses one named contribution. There is no bare current `data` and no
implicit `out = .` selection under a join.

**Uniform typed ports.** A transaction may emit on any declared output,
including `err`, `exit`, `expired` or application-specific names. Each
is checked by the same resolved `DataType`; tagged unions make finite
outcome spaces enumerable without an `error_set` side channel. Port
names do not close calls or classify failures. Genuine panics (bugs) are
supervision faults, never envelopes.

**Guest-language blocks in multi-input handlers.** In a single-input handler a
jq block's `.` is the input envelope; in a join there is no implicit
`.` — reference envelopes by port name (`a.data.x`, a Rastok expression) and scope
a jq block to one input when needed (`a | jq! { … }`). See §2.5 for the
languages themselves.

**Meta on a join.** `meta.sys` is always runtime-fresh on the emitted
envelope — its `parent_id` is the envelope that *triggered* the
transaction (envelope.md's cause table); the other contributors are
recorded in the journal, not on the envelope. The writable zones
(`meta.app`, `meta.ext`) merge BY VALUE, per field: absent everywhere
→ absent, one distinct value → kept, conflicting values → absent —
and an explicit write always wins (`meta.app.request_id =
a.meta.app.request_id` picks a side; two names keep both). A field a
merge may have dropped is statically optional: a required read
downstream refuses at compile time until an explicit write has
chosen the value (D-2026-08-10-2). Single-input handlers copy each
zone from their one parent; `meta.ext.content_type` follows the
payload, not the merge (D-2026-08-10-4).

**Lifecycle.** `on start` fires when the first envelope is admitted to a new
logical Node instance. A lifetime deadline or bare **`stop`** requests Closing
of that current instance after the transaction commits. `on stop` runs last,
after already admitted work and every required Connector terminal `exit` have
been handled under the bounded Closing contract. There is no targeted
`stop <name>`. To reset a `var` without ending its owner, assign it (`count =
0`). Stopping a Graph also closes its live descendants inner-first; this is a
lifecycle relation, not shared state.

**No `on tick`.** All time-based behavior comes from a `Clock`
wired to an input — time enters the graph as envelopes (§5):

```rastok
operator stats {
  instances = { key = meta.app.session_id }
  input  tick : timestamp                // a Clock tick's payload IS the timestamp
  var count: int = 0

  on event { count = count + 1 }         // scalar state update: Rastok expression

  on tick  {
    metric = { meta.app.count_per_min = count }
    count  = 0
  }
}

// wiring at graph level
connector minute_clock : Clock { every = duration("1m") }
minute_clock -> stats.tick
```

A join example (settle an order once both legs arrive, correlated by
the Operator's order-id instance):

```rastok
operator settle {
  instances = { key = meta.app.order_id }

  on (invoice, payment) {
    out = {
      meta.app.invoice_id = invoice.meta.app.invoice_id   // a join picks explicitly; common fields merge (see below)
      data = { order_id = instance.key,  // read-only current owner key
                      amount   = invoice.data.amount,   // envelopes are named by PORT
                      paid     = payment.data.confirmed }
    }
  }
}
```

The simulator controls every `Clock` like any other Connector. There
is no hidden `now()` inside an Operator or a mock; time is whatever the
most recent clock envelope carries. A real Connector may observe time
in an external system — for example, a Postgres query may call
`NOW()` — but its controlled Lab handler must return a declared outcome
instead of reading the host clock.

### 2.4 Node-local state

State is **read-modify-write, imperatively**, within a transaction. A
persistent field `var x: T = default` is read by its bare name (`x`)
— or `$x` inside a `jq! { … }` block (jq's own syntax for bound
variables). At the *start* of a transaction, `x` holds the value committed
by the previous transaction (or the default if uninitialized); assignments
mutate it **in place**, so a later read in the same transaction sees the new
value, exactly like an imperative function. The transaction as a whole still
commits atomically — state delta + emissions + audit — at its end.

```rastok
operator counter {
  instances = { key = meta.app.session_id }
  var count: int = 0

  on msg {
    count = count + 1                        // mutate ; reads below see the new value
    out = { meta.app.count = count }
  }
}
```

**Intrinsic computation uses Rastok expressions.** Their syntax and a
large operator subset are inspired by the
[Common Expression Language](https://cel.dev), whose reference behavior
remains a differential oracle for the compatible core. Rastok owns the
actual `DataType`/`Value` domain and extensions such as exact decimal,
unions and narrowing. Expressions are checked, bounded, pure and have no
ambient I/O. The right-hand side of an assignment is a Rastok expression
(§2.5): `count = count + 1`, `x = x * 2 + data.n`,
`label = meta.app.region + "-" + instance.key`. Structural reshaping may
use tagged frontends such as `jq! { … }` / `sql! { … }`. No expression
position admits `now()`, environment lookup, network or filesystem I/O.

**Emitting.**
- A port emits **once per transaction** — the value of its *final* assignment
  (last-write-wins); a port never assigned emits nothing.
- `out = { … }` with only reserved roots (`data`, `meta`)
  is the envelope form: it **inherits** the input (meta + data) and
  overrides only the listed fields — a *patch* ; a whole-`data`
  assignment makes the data fresh.
- `out = { … }` with no reserved root is a data record (§2.1): it
  inherits the input's `meta` and replaces `data` with the new struct.
- `out = .` re-emits the current envelope unchanged.
- `out = nothing` emits nothing on that port this transaction (the explicit
  form of simply not assigning it).

There is no `previous(x)` operator and no within-transaction snapshot trap:
a transaction reads the previous transaction's committed state at its start, then
proceeds imperatively. The synchronous-dataflow `pre` of Lustre/SCADE
applies at the **inter-transaction** boundary — each transaction is one cycle —
not *inside* a transaction (see §9.1).

**One owner, no remote reads.** A `var` belongs exclusively to the Node that
declares it and is visible only in that Node's handlers. A child Node cannot
read an ancestor Graph's mutable cell, and sibling Nodes cannot address one
another's cells by name. Keying a Graph serializes its own mailbox; it does not
turn the whole traversal of an envelope into a graph transaction.

Shared mutable behavior is modeled as an explicit stateful Operator with typed
command/result ports. Operations that must be atomic over the same state live
in that Operator's handlers and are serialized by its instance mailbox:

```rastok
operator account {
  instances = { key = meta.app.account_id }
  var balance: long = 0

  on debit {
    balance = balance - data.amount
    out = { balance = balance }
  }

  on credit {
    balance = balance + data.amount
    out = { balance = balance }
  }
}
```

This removes nominal action at a distance and gives one ACID unit everywhere:
the Node transaction. Across Nodes, composition is envelopes plus the causal
journal, not shared memory.

#### 2.4.1 Values and state: `val` / `var`

Rastok has only two keywords for named values, following the Scala
/ Kotlin / Swift convention: **`val` for immutable, `var` for
mutable**. The scope is determined by *where* the declaration sits,
not by the keyword.

| Keyword | Where | Mutable? | Lifetime |
|---|---|---|---|
| `val` | Unit/App-level | no | deployment run (global immutable input) |
| `val` | Graph-level | no | current Graph instance; lexically visible to descendants and may derive from `instance.key` |
| `val` | Operator/Connector-level | no | current Node instance; private to that Node |
| `val` | Inside a handler | no | One transaction (handler-local binding) |
| `var` | Node-level | yes (read-modify-write per transaction) | current logical Node instance; visible only in that Node's handlers |

```rastok
graph my_app : App {
  val DAILY_QUOTA   = 1000               // App-level constant
  val SLACK_CHANNEL = "#alerts"
}

operator pricing {
  instances = { key = meta.app.tenant_id }
  val TIERS = { "free": 0, "pro": 99, "enterprise": 999 }   // Operator-level constant (a map: `:` keys)
  var subscribed_tier: text = "free"                   // private Node-instance state

  on upgrade {
    val tier = data.new_tier                           // handler-local binding (val = DRY, used twice)
    subscribed_tier = tier
    out = { meta.app.price = TIERS[tier] }    // all of it is a Rastok expression
  }
}
```

A keyed Graph may hold **graph-level `val`s** derived from `instance.key`
(for example a session's home directory). Descendants may read those values
because they are immutable lexical context. A Graph `var`, if declared, is
still private to the Graph's own handlers; lexical containment never grants a
mutable read to a child.

These scopes also define evaluation phase. Unit/App boot `val`s are resolved
once for the deployment. A Node-level `val` is derived once for each new
instance from enclosing immutable values and that owner's local
`instance.key`; it cannot read `data`, `meta`, or mutable state. Handler-local
`val`s are transaction bindings. `instance.key` is a reserved contextual root,
not a name that searches upward: a child consumes a parent Graph key only
through a lexical Graph `val`.

Connector contracts apply the same distinction by declaring the latest
evaluation site a field permits: plan, boot, instance, or invocation. Checking
selects the earliest site where the written expression closes. Profile,
port-type, effect, behavior, and other plan-shaping attributes permit only
plan resolution. An operational setting (`cwd`, path, URL, command, arguments,
environment) may permit instance binding; its typed expression may consume
enclosing immutable context and is evaluated once when the Connector instance
binds. A literal still reduces at plan and an `env()`-only expression at boot.
The plan transports the actual phase and expression, so neither the runtime
nor the adapter guesses. This is what makes a child `Process` safely consume a
session Graph's immutable `cwd` without gaining access to its state.

Reading rules:
- **In bare DSL** (envelope literals, assignments, conditions):
  `val` and `var` are both read by their bare name (`X`).
- **Inside `jq! { … }` / `sql! { … }` blocks**: the guest language's
  own convention applies — jq uses `$X` for bound variables,
  SQL uses `?` parameters or named bindings depending on dialect.
- **Writing `var X`**: `X = <expr>` (assignment, bare).
- Both bare `X` (in DSL) and `$X` (inside jq) read the **same**
  previously committed value. They are the same operation
  expressed in the respective grammars.

Tunable runtime knobs (e.g., a feature flag that an admin can flip without
redeploying) are not shared variables. They are an explicit stateful Operator
with typed control and query/output ports:

```rastok
operator settings {
  var rate_limit: int = 100

  on admin {
    rate_limit = data.?rate_limit.orValue(rate_limit)
    changed = { rate_limit = rate_limit }
  }

  on read { current = { rate_limit = rate_limit } }
}
```

The admin pushes envelopes to `settings.admin` (via the Cloud UI or an HTTP
endpoint); consumers wire `settings.changed` or request `settings.read`.
Ownership and ordering are visible in topology instead of hidden behind a
nominal remote read.

### 2.5 Computation: Rastok expressions, tagged frontends, helpers, and primitives

#### 2.5.1 The core expression language and its guests

The DSL itself is deliberately **declarative** — nodes, ports, handlers,
wiring. Intrinsic computation uses Rastok expressions. Structural or
specialized computation may use registered tagged frontends with fixed
compilers/executors and typed boundaries. jq is the initial document
guest; SQL is the planned relational guest.
Additional guests are admitted only through the same contract.

**Rastok expressions — the intrinsic core.** Every expression position in the DSL — a
`where` guard, an instance-key expression, a Router selector, a state expression,
an attribute value, a simulation expectation — is one pure Rastok expression:

```rastok
on payment where data.amount > 100 && meta.app.region == "eu" { … }
count = count + 1
key   = meta.app.session_id
```

The expression language is *total*: it always terminates, has no side
effects, and cannot express loops. That is the point — every expression in a graph
can be type-checked, costed, and simulated without running anything.
Absence is explicit: `data.?discount.orValue(0)` reads a
possibly-missing field with a default; a *bare* read is accepted only
when the type proves the field present, never converted to a silent null.

**`jq! { … }` — document reshaping.** When the job is structural — build
a nested JSON body, filter and restructure a collection — a tagged jq
block hands its body to the jq guest:

```rastok
data = jq! {
  .data | { url:    "https://api.weather.example/v1/current",
            method: "GET",
            query:  { city: .city, country: (.country // "FR") } }
}
```

Inside the block, jq's own conventions apply: `.` is the input
envelope, DSL bindings are read as `$x`, and a missing field is `null`
(the jq tradition — it survives *only* inside jq blocks).

**`sql! { … }` — the planned relational view.** Data of type
`array<struct>` can be treated as a `table` (a view, not a type) and
queried with SQL — aggregations, group-bys, top-k — instead of
simulating relational algebra in jq.

The Rastok DSL is the host language. A postfix bang makes delegation
explicit: `json!`, `jq!`, `sql!`, `text!` and application-registered
names all select the tagged-frontend production without reserving their
identifier. The host never interprets a guest body; the registered
frontend supplies its framer/compiler and typed boundary. The build
produces a versioned `GuestProgram`, executed by a pinned
`GuestExecutor`. Each frontend/executor profile is fixed, every block has
typed input and output, and no source region has two lexical owners. jq
is the initial specified pure guest; SQL is planned and must
satisfy the same six-clause contract in
[stdlib.md](docs/spec/stdlib.md) (pure determinism, resource budgets,
typed outcomes — a guest program may never hang the graph).

The execution format is orthogonal to the source language. JavaScript, Rust
or another future guest may compile to Wasm before entering the
`ExecutionPlan`; a precompiled Wasm module may enter directly. A Wasm guest
with no I/O imports may execute inside an Operator. A Wasm program with
network, filesystem, process, clock or other external I/O executes behind a
Connector. Wasm provides a sandbox and ABI, not purity by itself.

#### 2.5.2 Helpers and primitives

Operators and Connectors look the same syntactically but differ in
nature:

- **Operators are the pure computation primitive.** The
  `operator <name> { … }` form expresses any computation node. The
  standard vocabulary names frequent patterns — `Router`, `Reducer`,
  `Aggregator`, `Joiner`, keyed Graph dispatch, and the jq-backed `Jaq`
  helper (cataloged with examples in
  [stdlib.md](docs/spec/stdlib.md)). `Sql` and the fuller window helper
  remain planned. A user
  may always fall back to the primitive form or contribute new helpers
  in pure DSL. (One-offs like a filter are just `Jaq` + `select` — no
  dedicated alias. Broadcast needs no `Fanout` alias because the atomic
  multi-emit statement `(a, b, c) = x` expresses it directly.)
- **Connectors are runtime primitives.** They bridge the graph to
  the outside world through Driver code (HTTP libraries, process
  spawning, database drivers, file I/O). A new Connector requires a
  semantic Contract and a registered native or sandboxed Driver; it cannot be
  defined in DSL alone. This is the *syscall surface* of Rastok.

In OS terms: Operators are userspace functions composed from DSL primitives;
Connectors are syscalls. The stdlib names, Connector contracts, `behavior`
rules and pure guest-program contract are specified in
[docs/spec/stdlib.md](docs/spec/stdlib.md).

**Effects on Connectors only.** Operators are pure by construction — they
compute, route, mutate state inside the graph, but they do not write to the
outside world. The exact effect classes, compensation rules and Saga behavior
belong to [docs/spec/effects-and-sagas.md](docs/spec/effects-and-sagas.md).

**Aspects** are orthogonal scoping behaviors composed on the `:` annotation
slot. The initial specified aspects are `Saga` and `Loop`; the generic mechanism is in
[docs/spec/aspects.md](docs/spec/aspects.md), with the aspect-specific
contracts in [effects-and-sagas.md](docs/spec/effects-and-sagas.md) and
[loop.md](docs/spec/loop.md).

### 2.6 Topology: static DAG with bounded dynamic dispatch

Rastok's topology is a **strictly acyclic directed graph** declared in
the DSL at compile time. There are no implicit back-edges. Every
connection from one port to another is visible in the source.

This is non-negotiable. The static DAG is what enables:

- **Path coverage testing** — every path from source to sink is
  enumerable from the topology.
- **LLM-driven graph generation** — the search space is bounded by the
  DSL grammar and the type system.
- **Model checking and theorem proving** — the topology is a finite
  object the verifier can analyze.
- **Backpressure** — every edge is a known channel with declared
  capacity.
- **Deterministic simulation** — the system can be closed under
  simulation because all I/O happens through declared Connectors.
- **Workflow soundness** — properties like proper completion and
  absence of deadlocks become decidable.

Dynamism is recovered through two bounded primitives, both of which
preserve static analyzability:

| Primitive  | Selection mode                       | Topology contract                        |
|------------|--------------------------------------|------------------------------------------|
| Router     | Runtime choice 1-of-N (or list-of-N) | All N output ports are declared          |
| Dispatch   | Keyed sub-graph instance             | The sub-graph and key expression are statically declared |

Broadcast to multiple outputs is expressed inline by the atomic
multi-emit statement `(a, b, c, d) = x` — the same envelope is
delivered on every listed output port. This subsumes a dedicated
`Fanout` alias and composes naturally with conditional logic without
requiring an upstream split.

A Router selects one or more statically declared outputs:

```rastok
operator route : Router {
  select = data.kind
}

route.invoice -> handle_invoice
route.refund  -> handle_refund
```

Here, `select` is a Rastok expression. The runtime value chooses a route, while the
possible ports remain visible to the compiler. A list of port names
selects several outputs.

Dispatch is the keyed instantiation of a declared sub-graph:

```rastok
graph conversation {
  instances = { key = meta.app.session_id }
  lifetime = { kind = idle ; after = duration("2h") }
  . -> classify -> answer -> .
}

requests -> conversation -> responses
```

The topology contains one `conversation` declaration. At runtime, each
session key receives its own instance and state.

In each case, *what* happens at runtime is dynamic. *Where* it can
possibly happen is statically declared. The compiler can therefore
enumerate paths and check types — and future verifiers can target
invariants (§6.1) — yet the graph can make real-time decisions based on
envelope data.

**Where did the loops go?** A strict DAG raises an immediate objection:
real systems iterate. Rastok's answer is that cycles never appear *in
the topology* — they appear in three other places, each bounded and
journaled:

- **In state and sequence.** A stateful Operator accumulates across arrivals
  in its logical instance (§2.4, §3) — what a `while` loop does with a mutable
  variable, Node-local state does across envelopes.
- **In the Connector** (§2.7). External feedback loops — a user replying
  turn after turn, a retried HTTP call — stay outside the graph; each
  round trip re-enters as a fresh arrival.
- **In the `Loop` aspect.** Bounded self-iteration — "refine until the
  score passes, at most 10 times" — is declared by annotating a node
  `: Loop` and configuring its well-founded measure:

  ```rastok
  graph refine: Loop {
    Loop.variant = {
      kind = measure
      initial = 10
      next = value - 1
      until = value <= 0
    }
    …
  }
  ```

  `Loop.variant` is an attribute of the `Loop` aspect, not a `var` in
  Operator state and not the dynamic data type named `variant`. The
  fields on the right are Rastok expressions; `value` is the current
  measure bound by the `measure` value.

  The runtime re-injects the node's output as its next input (trigger
  `l:next`) while the exit condition is false — an *event*, not
  a back-edge. Every application is journaled and the DAG stays
  acyclic. The runtime checks strict decrease; a violation is the typed
  error `LoopVariantViolation`. A simple counted measure gets a
  compiler-derived bound. Other forms declare an explicit `limit`, so
  a convergence claim cannot become an unbounded production loop
  ([loop.md](docs/spec/loop.md) §2, §12).

The maxim form: *cycles do not appear in the graph — they appear in
state, in events, and in sequence*. Every `Loop` carries a decreasing
measure and a finite call bound. The runtime rejects a non-decreasing
step, and a future verifier may discharge selected obligations
statically.

### 2.7 The Connector as cycle absorber

Real-world systems are intrinsically cyclic: a Codex agent exchanges
turns with the user over hours; an HTTP service retries on failure; a
database transaction commits then is read back. Rastok absorbs all of
this cyclicity inside Connectors.

A Connector encapsulates the external state, the external timing, and
the external feedback loop. From the graph's perspective, a Connector
has typed input and output ports — no cycle is visible. The graph
remains acyclic. This is what makes simulation, testing, and
verification mechanically possible.

What the Connector absorbs is the **cycle, not the nondeterminism**: the
external side answers with its own timing, and a response re-enters the
graph as an arrival whose order relative to other arrivals is a race.
Those arrival orders are recorded per hop in the journal — which is
exactly what makes replay deterministic (envelope.md §2.0/§8). The
Connector draws the boundary; the journal captures what crosses it.

**The Contract is not the Driver.** Source names resolve through a compiler
catalogue to a stable versioned contract id such as `rastok/process@1`. The
Contract declares inbound/outbound roles, typed ports, attributes and
lifecycle rules. A Driver is one physical implementation of those semantics.
Before execution, a runtime registry binds every active Connector Node by the
exact id and proves that the implementation exposes every required role.
Neither crate names, shared-library paths, build features, Hosts nor Workers
enter the plan or causal identity.

```text
RKG Connector ──▶ Contract catalogue ──▶ validated plan
                                                (id + roles + exchange + facts)
                                                │
static factories ──▶ Driver registry ──bind─────┘
future plugin host ─┘                    │
                                        ▼
                               one VM + one journal
```

Static composition is the implementation available today: embedded, minimal
and full standalone builds may link different Driver sets while consuming the
same plan model. A future plugin sidecar can publish a data-only capability
manifest — canonical contract ids and roles — before code is loaded, then
register an ordinary factory through the same boundary. Rastok does not yet
freeze a native plugin ABI, artifact format, trust policy or hot-reload
lifecycle. Discovery is physical assembly, not a second execution model.
The reference CLI makes that assembly visible: Process is linked by default,
an explicit `--file-root` adds File and supplies deployment data that never
enters the graph or plan identity, and a hosted-HTTP build links HttpClient
without changing its contract id.

Hosted network Drivers use the same separation. The synchronous Driver/VM
edge feeds one bounded private pump: finite work and result queues, a separate
priority control lane, and a shared byte budget. HTTP/1.1 and HTTP/2 currently
target Hyper with Rustls; HTTP/3 targets quiche with BoringSSL; WebSocket
messages target Tungstenite. Those are replaceable physical bindings, not
semantic Node types. They are enabled by static build features and all async,
socket, TLS and library values remain below the Connector module. The two TLS
stacks deliberately consume one exact compiled root corpus and the same
in-memory identity encodings. A minimal or embedded build omits them without
changing a plan. HTTP/1.1 Upgrade is not evidence for the distinct HTTP/2 or
HTTP/3 Extended CONNECT adapters.

The live HttpClient demonstrates the boundary end to end. One canonical
descriptor seals method, URL, lossless ordered headers, exact body and
idempotency material before the call opens. HTTP versions are attempted in
declared order only until request bytes may be visible; after that point a
failure is terminal rather than an unsafe implicit retry. H1 uses origin-form,
H2 multiplexes calls on a pooled connection, and H3 uses verified quiche TLS.
All usable statuses return an ordinary typed body plus status/header/version
observations and one terminal exit. Finite local saturation leaves the
journaled call pending; it is pacing, never a fabricated network failure.

HTTP schema intent remains typed without pretending the registry of media
types is closed. `media_type` is an open logical scalar with a canonical text
carrier; standard request fields live in a sparse typed `headers` struct,
while uncommon extension fields live in `custom_headers`. Driver-owned and
managed fields cannot be overridden through that extension map. Responses
retain every header byte in `meta.ext.http_headers` and additionally project
valid standard facts such as canonical `meta.ext.content_type` for ordinary
typed graph logic.

OpenAPI is a build-tool input, not a second runtime type system. The project
resolver discovers static HttpClient/HttpServer endpoints, projects selected
OpenAPI 3.0/3.1 component schemas through Rastok's one `DataType` algebra, and
writes one deterministic generated Rastok source. Unsupported or lossy schema
features refuse by path instead of becoming `variant`; authored files are
never rewritten. This makes the normal LLM-assisted workflow resolve, inspect,
check and commit generated types while preserving a textual review boundary.

The Lab follows the same rule. A mock Driver binds under the same id and roles,
uses the same family request protocol and resolved result types, and replaces
only the post-journal external interaction. It never replaces the VM or its
bookkeeping. Each Connector family owns that shared pure request protocol;
the Lab composes family-provided binders instead of dispatching on contract
names.

**Lifecycle-bound exchanges do not invent hidden addressing dimensions.** A
static Connector Node may run at several logical addresses, and one Node
instance may admit several exchanges concurrently when its resolved policy
permits. There are three cardinalities behind the same two Driver roles:

```text
event    external fact ──▶ one inbound root; no per-event exit
call     operation ──▶ zero or more outputs ──▶ exactly one exit
session  open ──▶ messages in both directions* ──▶ exactly one exit
```

Clock ticks and calendar instants are events. Every call has a causal
call identity. For a world-originated Call, the committed request root itself
is that identity and opens the obligation on the exact source Connector
instance; its sole terminal fact is an ordinary `l:reply` transaction, not a
second outbound call. Every session has a causal
logical session reference independent of its socket, negotiated transport and
physical placement. A Process job or database operation is a call; a
WebSocket connection and the future Process service profile are sessions.
Neither automatically invents a Node-instance key.

`stop` and `exit` name the two distinct boundaries. `stop` closes the current
Node instance; `exit` reports the observed termination of
one admitted exchange. Many exits may occur before one stop. Closing a Node
instance stops admission and must drain or cancel every open call and
session under a bounded policy; it cannot erase their terminal obligations.
Call identity correlates call outputs. Session identity correlates all
messages under one open resource; a WebSocket message does not receive a fake
per-message exit. No output may follow the corresponding terminal `exit`.

For a `Process`, `Exited { exit_code, signal }`, `TimeoutExpired`,
`SpawnFailed` and the former typed error codes are cases of `ProcessExit`.
`out` stays the data stream and `Process.err` stays stderr; a graph reacting
to termination wires `process.exit -> …`. A resident resource has a separate
runtime `run_id`; `restart = { … }` may create a fresh resource run inside the
same logical Node instance without inventing another source key. `meta.sys`
stays closed while call, resource, and Node-instance identities remain causal
journal facts.

---

## 3. Instances, Keys, and Lifetime

The old word “partition” bundled three different questions:

1. **Where does this envelope belong?** — logical instance addressing.
2. **When does that instance end?** — lifetime.
3. **Who can read mutable state?** — ownership and lexical scope.

Rastok keeps those concerns separate. A Node owns its address selection, a
separate lifetime may close it, and every mutable cell remains private to the
Node that declares it. The normative detail is
[docs/spec/instances.md](docs/spec/instances.md).

### 3.1 One Node, optional key, optional lifetime

Every Graph, Operator, and Connector may carry the same common attributes:

```rastok
operator cart {
  instances = {
    key = meta.app.customer_id
  }

  lifetime = {
    kind = idle
    after = duration("30m")
  }

  var total: decimal(18, 2) = 0
  ...
}
```

The attributes are independent:

| Key | Lifetime | Meaning |
|---|---|---|
| absent | absent | persistent singleton |
| present | absent | persistent instance per key |
| absent | present | temporal singleton/window within the enclosing Graph address |
| present | present | keyed session/window |

There is no named or inline `partition`, no `per` binding, and no catalogue
of standard partition names. The owner states its key once. Absence of
`instances` is an explicit singleton in the enclosing Graph instance;
absence of `lifetime` is persistent.

At the App root, this makes a global tumbling window honest instead of
simulating one with a fake constant key:

```rastok
operator global_metrics {
  lifetime = { kind = tumbling ; every = duration("5m") }
  var total: long = 0
  ...
}
```

### 3.2 Key selection is a journaled routing decision

`instances.key` is one Rastok expression evaluated exactly once when an
envelope crosses into that owner. It may read the complete immutable entering
envelope — `data`, `meta.sys`, `meta.app`, and `meta.ext` — plus immutable
`val` bindings from enclosing lexical scopes. The owner's body is entered only
after selection, so its own `val`s cannot participate in its key. It cannot read target mailbox
state, arrival sequence, commit time, selected shard, or another destination
fact that does not exist yet.

The M1.3 port solver first resolves every effective input type. The compiler
then infers one total key type under every input that can reach the Node.
Missing paths or incompatible inferred types refuse; omission of a source port
annotation never silently becomes `variant`. A world-producing source uses
its Connector contract's resolved ingress-produced envelope type as the same
admission signature; having no graph input is not an untyped exception.

A key is either a canonical map-key scalar/enum value or a closed, recursively
nested struct of such leaves:

```rastok
instances = {
  key = {
    tenant = meta.app.tenant_id
    conversation = data.conversation_id
  }
}
```

Field order is not identity; the address frame seals the complete resolved key
type beside its value, so field name, resolved type, and canonical value are.
Optional/missing leaves, floats, dynamic `number`, null, variant, union,
collections, tables, and opaque values refuse. The same canonical bytes serve
routing, journal identity, state lookup, and future shard hashing.

The producer reserves destination capacity, obtains the current generation,
and records the selected target instance reference `(address, generation)`
before committing. Replay uses that recorded decision and never re-routes old
traffic by evaluating the expression again. Rewriting metadata later cannot
migrate the current instance, and delayed transport cannot enter a successor
generation.

Nested entry is ordered outermost to innermost. An enclosing owner is selected
and its immutable instance context is validated before a child may use that
context in its own key or Connector configuration. A source Connector's
listener or poller must therefore be boot-configurable; instance
configuration starts only after an accepted envelope has selected that
instance.

The selected key is contextual `instance.key`, not
`meta.sys.instance_key`. `meta.sys` belongs to the immutable envelope and
travels across Nodes; an envelope can visit several owners with different
addresses. Hop-local addressing therefore belongs in the journal and the
current execution context. `instance.key` is statically available only on an
owner that declares `instances.key`; it never means an ancestor key or null.

### 3.3 Logical address, state ownership, and joins

A logical address is:

```text
static Node path
+ dynamic key/time frames of keyed or temporal ancestors
+ this owner's dynamic frame, if any
```

Frames are ordered outermost to innermost. An instance is the currently live
mailbox, derived immutable bindings, Node-local state, open calls, armed events,
and lifecycle at that address. After closure, a later delivery may create a clean instance at the
same address. The runtime calls successive lives **generations**; generation
is the zero-based journal-derived ordinal of `l:start`, fences stale events and
deliveries, and never enters the logical address or call identity. Runtime and
journal facts refer to an instance with `(address, generation)`.

A `var` belongs only to its declaring Node and is visible only in that Node's
handlers. Keying a Graph does not create a transaction around the Graph and
does not expose mutable cells to children. Graph-level `val` bindings remain
lexical and immutable, so descendants may safely consume context derived from
the Graph's `instance.key`.

When mutable behavior must be shared, it is an explicit stateful Operator with
typed command/result ports. That topology makes ownership, serialization, and
failure visible; there is no nominal cross-Node state lookup.

A join correlates inside the logical instance of its owning Operator.
`where` refines candidates and `within` / `last` bound retention. There is
no join `by` override: a different correlation domain is expressed by the
Operator's own `instances.key`, keeping routing and correlation under one
authority.

### 3.4 Lifetime and Closing

Lifetime uses the ordinary composite-attribute form:

```rastok
lifetime = { kind = persistent }
lifetime = { kind = idle       ; after = duration("2h") }
lifetime = { kind = fixed      ; after = duration("30s") }
lifetime = { kind = single_pass }
lifetime = { kind = tumbling   ; every = duration("5m") }
```

- **persistent** arms no deadline;
- **idle** closes after mailbox-admission silence;
- **fixed** closes at a fixed age from the creating admission;
- **single_pass** admits one root and closes after its bounded work settles;
- **tumbling** selects the epoch-aligned processing-time box open at admission.

Sliding windows and event-time/watermark semantics remain deferred because
they make one arrival select several boxes or require an additional time
authority.

The first admission starts an instance causally with `l:start`. Bare
`stop` requests Closing of the current Node instance after the current
transaction commits; it has no target operand. Closing stops external
admission, continues already admitted internal work for a finite drain,
cancels and settles remaining calls without suppressing their typed `exit`,
runs `on stop` after the last required terminal fact, then reclaims state.
The exact policy is
`closing = { drain_for = <positive duration>; settle_for = <positive duration> }`.
Both fields are required whenever a finite owner's subtree can hold a Process
call; there is no hidden action, kill, grace period, or timeout default.
The first close request is a durable `l:closing` fact. Arrivals from outside
that closing scope reserve the exact successor generation in its ordinary
bounded mailbox; no second queue exists. At the drain deadline, remaining
internal deliveries are recorded as consumed by the eventual stop and calls
are cancelled in canonical reference order. A completion observed before the
settlement deadline publishes its ordinary typed exit. Otherwise the outbound
record closes `abandoned`, an empty `OutcomeUnknown` exit is emitted, and a
later reply is a non-causal discarded observation.

Stopping a Graph also closes every live descendant created in that Graph
generation, inner-first. This is a durable lifecycle relationship, not shared
memory. A non-root child's `l:start` carries its enclosing Graph instance
reference because the logical address intentionally omits ancestor
generations. New arrivals wait under normal bounded backpressure and start a
fresh generation after reclamation.

Obligation scope matters: a pending Loop `l:next` wakeup is generation-bound
and becomes stale when its instance closes; Saga compensation debt is a fold
over the journal and survives instance closure, restart, and shard movement.

### 3.5 Concurrency and future sharding

One logical instance reference owns one bounded mailbox; at most one worker
executes a transaction from it at a time. FIFO is guaranteed within that
reference. No
order is promised across independent addresses, and Node-private state makes
that non-order unobservable as shared memory.

Hierarchical address does not force hierarchical placement. Every keyed owner
is an independent routing boundary and may map to a virtual shard independently
of its parent. Otherwise one hot outer key would trap every inner key on one
shard. Crossing such a boundary remains a journaled delivery even when both
owners are currently colocated.

```text
logical instance address
        -> stable virtual shard
        -> current Worker owner
```

Worker count may change without changing semantic identity or the virtual
shard function. Changing the virtual-shard count is an explicit reshard
migration. Worker, Host, lease, and leader epoch are never causal fields.
The synchronous one-worker runtime is the embedded/reference realization of
this same contract, not a different language model.

## 4. Distribution Model

Rastok is designed for three execution scales, with the same `.rkg`
source remaining portable across them:

| Mode | Use case | Process model |
|---|---|---|
| `embedded` | edge AI, IoT, mobile, CLI tools | Single process, RAM only |
| `standalone` | single VM/container, small SaaS | One Host with Coordinator + Workers, local persistence available |
| `clustered` | multi-tenant SaaS, enterprise | N Hosts, key-sharded, replicated persistence |

A `.rkg` App should not encode assumptions about one of these modes;
runtime mode and storage belong to deployment.

> **Design intent, not an implementation claim.** The initial runtime
> target is single-process execution. Standalone persistence and the
> `clustered` model below — membership, leases, epoch fencing, shard
> rebalancing, and cross-host routing — require separate contracts.
> They are described here only to keep the conceptual model coherent.

### 4.1 Runtime topology

```
Cluster
└── Host                         (machine or pod)
    └── Runtime                  (one App deployment)
        ├── Coordinator          (parent/control process)
        └── Workers              (bounded child data-plane processes)
```

**Host** is the machine-level unit. “Node” is reserved for graph elements —
Operators, Connectors, and Graphs — never machines.

The target standalone/clustered runtime follows a supervised multi-process
shape. The Coordinator owns membership, leases, virtual-shard placement, and
Worker restart. Each Worker loads the same validated App plan and executes the
logical addresses whose shards it currently owns. A Worker crash therefore
does not have to kill the Host runtime: the Coordinator fences it, restarts or
reassigns its shards, and recovery folds the journal permitted by the declared
durability scope.

The reference M1 runtime is deliberately the degenerate form: one process,
one Worker, synchronous pump. An embedded/no_std deployment may remain exactly
that. Neither source semantics nor journal identity depends on the existence
of an OS process boundary.

An inbound Connector is a logical producer with its own stable replica
provenance. Whether its Driver physically lives beside the Coordinator or in a
Worker is a deployment choice. The Driver proposes a draft; runtime authority
assigns its sequence, mints the complete inbound envelope, evaluates the first
target address, journals that route, and only then delivers. Workers perform
outbound effects through bound Outbound Connector Drivers. A child spawned by
the `Process` Connector is workload isolation, not itself a Rastok Worker.

### 4.2 Durability scopes

Every stateful Node has one effective durability scope:

| Scope | Survives | Storage shape | Typical uses |
|---|---|---|---|
| `none` (default) | nothing beyond the process | memory only | counters, caches, temporary aggregation buffers |
| `local` | Worker process crash | local durable journal | sessions, deduplication sets, recovery on one Host |
| `global` | Host failure | replicated durable journal | sagas, financial workflows, business-critical state |

Durability inherits through the graph tree. The closest explicit
declaration wins:

```text
Node declaration > closest parent Graph > App > none
```

An App can therefore set a default once, a nested Graph can strengthen
or weaken it for one region, and an individual Node can override that
region:

```rastok
graph checkout : App {
  durability = local

  graph payments : Saga {
    durability = global

    connector bank : HttpClient { ... }  // effective: global

    operator short_lived_cache {
      durability = none                   // explicit local override
      ...
    }
  }
}
```

The override is deliberate, not silent. The compiler reports every
place where a child weakens its inherited guarantee.

**Compile-time coherence rules**:

1. **Runtime mode support**: `durability = global` requires runtime
   mode `standalone` (with a global backend configured) or
   `clustered`. In `embedded`, this is a **build error**. No silent
   degradation.
2. **Weakening report**: an explicit child scope below its inherited
   scope emits a warning and appears in the coverage report.
3. **Chain propagation**: if a durable Node emits to stateful work with
   a weaker effective scope, the compiler warns that the downstream
   guarantee is not preserved.
4. **Audit coverage**: the compiler emits a coverage report
   ("X% of nodes are global → full audit / partial audit on nodes
   [list]").

**Provenance is not a separate feature.** When every Node in the
graph has `durability = global`, each transaction's state delta —
including the envelope ids that triggered it and those it emitted
— is persisted globally. This is the audit trail. Setting
`durability = global` everywhere is the same as enabling full
provenance.

**Durability is the runtime's own, never a dependency.** The DSL
declares the *intent* (`durability = none / local / global`); the
runtime provides it. For `local`, the runtime needs nothing installed
beside it: **the journal is its own write-ahead log** — append-only
binary segments owned and written by the runtime itself, running
identically on a laptop, a Raspberry Pi, or an air-gapped server.
There is no database to deploy, no storage engine to configure, no
external system to operate in order to be durable.

External systems appear in a Rastok graph in exactly one role:
**Connectors**. Postgres, Kafka, S3 and their peers are things an
*application* talks to — never hidden application dependencies of the
runtime. The `global` scope uses a replication backend selected by
deployment. That backend is an implementation and operations choice,
not part of an App's graph semantics.

Persistence follows evidence, not aspiration. The initial runtime uses
`durability = none` while preserving the journal-shaped contract.
`durability = local` and standalone persistence follow only after that
contract survives crash and recovery testing.

> **Reminder:** the Rastok model does not require an application
> database. The DSL knows about `none / local / global` durability
> scopes; the runtime owns its journal; application stores are
> Connectors.

### 4.3 Scaling and sharding

Horizontal scaling is graph-plan replication plus assignment of **stable
virtual shards**. The mapping has three layers:

```text
logical instance address
        -> stable virtual shard
        -> current Worker lease
```

The canonical logical address is the static Node path plus its outer-to-inner
dynamic key/time frames (§3). A fixed, architecture-independent hash maps
those bytes into a deployment's virtual-shard space. The virtual-shard count
is stable and independent of Worker count: adding a Worker moves leases, not
keys. Changing the virtual-shard count is an explicit reshard migration with a
separate compatibility protocol.

Each keyed owner is a routing boundary and may shard independently from its
keyed parent. This is essential: a hot tenant must not force all of its
sessions onto one Worker. The address remains hierarchical for lifecycle and
causality, but placement is free to distribute the child frame. An unkeyed
child remains the singleton child of its enclosing Graph address.

Crossing a keyed boundary is always an ordinary journaled delivery. The
producer reserves and seals the target instance reference — logical address
plus generation; the transport consults the current lease and obtains a
durability receipt. If source and target are colocated,
the runtime may use an in-process fast path but may not remove causal records.
No Worker, Host, lease, epoch, or target shard id enters the envelope or causal
journal.

A committed route consumes the referenced generation even if physical
delivery never reaches a worker. Recovery either completes that generation's
start or Closing retires it with an explicit residual-delivery fact; a later
instance can never reuse an ordinal still named by history.

A Graph close can therefore cross Workers. Parent-to-child lifecycle relations
are journal-derived obligations: the parent sends durable close requests to
its live children and completes inner-first. The local runtime executes the
same protocol as function calls.

For globally durable keyed state, the availability posture is CP: one virtual
shard has one fenced writer. On Host failure it becomes unavailable until a
new lease is established and state is rebuilt from the replicated journal.
Lease epochs reject stale writes at the durable boundary. Membership, lease,
rebalancing, and reshard protocols remain future cluster work; this section
fixes the semantic constraints they must preserve.

There is no source-level arbitrary Operator placement. Authors declare
logical addressing, not machines. Workers all understand the same plan;
placement policy assigns virtual shards and may later offer colocation hints
that cannot alter graph behavior or identity.

---

## 5. Time and Sequence

Rastok keeps four distinct notions of "time" cleanly separated. Not
to be confused with the synchronous languages (Lustre, Esterel,
SCADE) from which Rastok inherits ideas: those unify these notions
under a single global atomic clock, while Rastok exposes them as
orthogonal mechanisms.

| Concept              | Definition                                             | Where it appears                              |
|----------------------|--------------------------------------------------------|----------------------------------------------|
| **Sequence**         | FIFO order of envelopes on a single input              | Stateful reads (`x` carries history)          |
| **Wall-clock**       | Real-world time, delivered as envelopes by a Clock | Timeouts, throttling, scheduling, periodic ticks |
| **Causality**        | Happens-before partial order on the global trace       | Theorem reasoning                            |
| **Simulation clock** | Virtual clock controlled by the simulator              | `simulate at 50ms`, replay                   |

Instance lifetimes combine a declared duration with journaled supervision
events.
The duration is a pure value. Expiration occurs only when a Clock event
advances the relevant timeline.

### 5.1 Sequence

State is read-modify-write, imperatively, within a transaction. A field
`var x` is read by its bare name `x` (or `$x` inside `jq! { … }`).
At the start of a transaction `x` holds the value the previous transaction
left; assignments mutate it in place, so later reads see the new
value. The transaction commits atomically at its end. Each transaction is one
cycle — the across-transaction "value the previous transaction left" is the
Lustre/SCADE `pre` (§9.1), but *within* a transaction the sequence is
imperative. See §2.4.

### 5.2 Wall-clock time

Start from the failure mode. A node that calls `now()` reads a value
that appears in no journal and no test fixture — two runs of the same
inputs diverge, a replayed incident does not reproduce, and a test can
only *wait* for a timeout instead of *declaring* it. One hidden clock
read anywhere in a codebase is enough to destroy reproducibility
everywhere.

Rastok's rule closes the hole by construction: **the graph never reads
time — time arrives as data.** There is no `now()` callable from a Node,
no `on tick` builtin, no implicit timer, *and no invisible runtime clock
either*.

The reasoning is the same as for HTTP or a database: wall-clock time is
a value the program does not control — it is produced by the world
outside the graph. And in Rastok, everything that comes from the outside
world crosses a declared Connector (§2.7). **The clock is the outside
world, so a clock is necessarily a Connector** — visible in the source,
journaled at its boundary, and mockable in simulation like any other
external system. This is not a stylistic preference; it is decision 2
of §1.3 applied to time.

A Node that needs to react to time therefore has an input wired to a
`Clock`, and receives ticks as ordinary envelopes whose payload is the
timestamp — the `stats` Operator of §2.3 is the canonical shape:

```rastok
operator stats {
  instances = { key = meta.app.session_id }
  input  tick : timestamp        // time is just a port
  …                              // on tick { … } — an ordinary handler
}

// wiring lives at graph level — the port stays decoupled from its source
connector minute_clock : Clock { every = duration("1m") }
minute_clock -> stats.tick
```

"What time is it?" becomes "what did the last tick carry?" — a
question about received data, answerable identically in production, in
replay, and in simulation.

**Wiring is always graph-level, never inside a node body.** A node body
declares **what the node is** (its ports, handlers, effects); the graph
section declares **how nodes compose**. A shared source is reused by
adding edges, not by touching the nodes:

```rastok
minute_clock -> a.tick
minute_clock -> b.tick
```

(There is no port-side `<-` shortcut. The explicit form keeps node
bodies pure and lets simulation overlays rebind sources without
touching node interfaces.)

**The stdlib `system_clock`.** A `Clock` named
`system_clock` is provided by the stdlib at App boot time. It
drives the runtime's lifetime management (idle TTL expiration,
window rollover, fixed TTL) — but it is *not* a hidden internal
clock. It is a real Connector, visible in the deployment manifest,
mockable by the simulator like any other Connector. In simulation,
advancing virtual time advances `system_clock`, which fires all
lifetime expirations deterministically.

For most patterns "do X when time passes", you do **not** need an
explicit `Clock` wiring:

| Pattern | Mechanism (no clock wire needed) |
|---|---|
| Cleanup of an idle Node instance | `lifetime = { kind = idle; after = duration("2h") }` |
| Tumbling window aggregation *(sliding windows are deferred)* | `lifetime = { kind = tumbling; every = duration("5m") }` |
| Rate limit "max N per minute" | keyed or singleton tumbling instance + a private counter |
| Deadline / timeout for a saga | the distinct `Saga.timeout` aspect attribute |

Explicit `Clock` is reserved for patterns where an
Operator actively emits or polls on tick — periodic metrics,
external state polling, heartbeats.

Production and Lab execution differ only in the source of clock
events. In production, `system_clock` emits events derived from real
time. In the Lab, `advance duration("2h")` asks the virtual clock to
emit the same relevant events without waiting two hours. Those events
travel through the same lifetime, window, and handler logic.

For example, an idle lifetime records the last mailbox-admission event and its
declared duration. Advancing the Lab clock past that deadline emits the
same expiration event that production would emit later. The resulting
`on stop` transaction is therefore deterministic for the same plan,
journal, and clock events. Rastok does not claim that real time itself
is deterministic; it makes every time observation explicit and
replaceable.

### 5.3 Causality

Causality is derived from the composition of inputs and emissions.
For a *concrete run*, the runtime and replay tooling reconstruct the
happens-before relation from the journal (`parent_id` chains plus the
per-hop records) — there is no causal token attached to envelopes. A
*future theorem* does not walk traces: it reasons over the transition
system a simulation closes, quantifying over all admissible
trajectories — a reserved surface whose body is not yet specified
(§6.1).

### 5.4 Simulation clock

The simulator owns a virtual timeline. It dispatches injected
envelopes at declared virtual times, drives mocked Connectors
(including `Clock`) accordingly, and asserts on the
observed output trace.

This is what allows Rastok to be asynchronous in production yet
synchronous-like under simulation: the simulator closes the graph
under controlled inputs and replays deterministically.

---

## 6. The Lab: Simulation, Experiment, and Verification

Every App may declare its twin: the **lab** — at
most one *logical* lab per App, possibly split across same-name
fragments in several files, bound to `graph <name>: App`. It is the App's
living corpus of tuning and assurance: everything used to stimulate,
observe, reproduce, experiment, debug, and eventually verify the App
lives there, and none of it deploys.

Inside the Lab, **simulation remains the first-class artifact** — Rastok
is simulation-first. A graph declares the machine Rastok runs; a
simulation declares the controlled world around that machine — inputs,
Connector behavior, virtual time, faults, observations — and it is
executable deterministically. Experiment and theorem are its siblings,
not its extensions. An experiment runs selected real boundaries
against the real world (§6.3); a theorem is a declarative, universal
claim whose surface is reserved (§6.1). In the long run, the simulation
is the primary specification: finite scenarios under-determine a graph,
but a graph is written — by a human or an LLM — *to satisfy them*, and
stays constrained by them (§6.5).

The ontology is a mirror:

| Production | Lab twin |
|---|---|
| App | Lab |
| Graph | Simulation — closes it (`on app`) |
| Connector | Mock Driver — replaces the external interaction, never the bookkeeping |
| deployed App + real profile | Experiment — meets the real world |
| a concrete execution | Scenario |
| an observation | Expect |
| Graph + Simulation | ⊨ Theorem *(reserved for future use)* |
| Operator | **itself** — pure logic is never mocked: it runs |

The key is **same program, different world**. Pure Operators run
unchanged. Connectors mark typed effect boundaries. An App execution
resolves effect intents against real systems; a simulation in the Lab
resolves the same intents through controlled handlers, virtual time,
and scripted faults.

The following is an explanatory engineering shape, not DSL syntax:

```text
step(plan, state, event) -> (new_state, effect_intents)

App execution  : effect_intents -> real outcomes
Lab simulation : effect_intents -> controlled outcomes
```

Outcomes return through the journal as events. Rastok does not pretend
that effects are pure: it makes them explicit, typed, isolated, and
substitutable while preserving the same graph semantics.

A *fixture* is deliberately absent from that table: it is not a
first-rank object but the pre-written outer trace of a Scenario —
injections, mock replies, time advances, faults — playing the world-half
of the production journal. A lab can span several files (same-name
fragments merge into one namespace) and never alters the production
graph or its `graph_version`.

### 6.1 The layers of assurance

The Lab separates **declarations** (simulation, experiment, scenario,
mock, theorem) from **operations** (simulate, experiment, challenge,
prove). Each operation answers a different question:

```
simulate      →  run the declared scenarios — deterministic, fast feedback
experiment    →  run the real App against at least one real boundary,
                 in real time; evidence from one run, never a
                 universal claim
challenge     →  try to refute a theorem within declared bounds;
                 best verdict "no counterexample within bounds,"
                 never "proved" unless those bounds cover the state space
prove         →  the future operation that attempts to DISCHARGE a
                 theorem (verdicts below)
```

`theorem` itself is **not** an operation: it is the reserved
*declaration* — `theorem <name> over <simulation> { … }` — of a
universal property over all admissible trajectories of
(graph + simulation). And « all admissible trajectories » must itself
first be defined (input domains, nondeterminism, faults, time,
fairness — the prerequisite rung of the ladder) before `prove` can
mean anything.

Future verdicts are epistemically exact — `proved`, `proved under
assumptions […]`, `refuted` (with a **minimal counterexample exported as
a replayable Scenario**), `no counterexample within bounds`, `unknown`
(with the opaque frontier — a guest program, an LLM, a partner —
precisely identified). An opaque component never yields a false
`proved`; a theorem may still hold *for every possible output* of the
opaque block.

The assurance layers do not substitute for each other: **a theorem
reasons about a graph under declared Connector contracts; an experiment
checks whether real Drivers and external systems still honor those contracts.**
Evidence grows incrementally:

```
scenario → simulation → experiment → challenge → proof
```

A team starts with scenarios (acceptance tests) and grows toward
theorems (formal invariants) as a system matures — without rewriting
anything along the way.

### 6.2 Simulation as PM/PO specification

A simulation block describes a possibility space around a graph in a form a
non-developer can read and write. §1.5 already showed the full shape on the
approval flow — a mocked partner, injected sources, `advance duration("48h")`, a typed
outcome asserted on `decide.expired`, an `expect nothing`. Its scenarios are
acceptance criteria a product owner can read: "the decision may arrive
first," "a rejection is a business outcome," and "no decision within
48 hours becomes the typed `JoinExpired` outcome." Nothing waits
forty-eight real hours: `advance` moves the *virtual*
clock, and every time-dependent decision in the graph (join expiry,
Connector timeouts, lifetime and window closes) fires deterministically —
§5's "the clock is a Connector" paying off.

Key properties:

- **A simulation is the graph's mirror, not a separate test language**:
  the graph says how the system reacts to envelopes; the simulation says
  which world surrounds it — inbound inputs, mocked Connector behavior,
  virtual time, failures, and expected observations.
- **A simulation lives in the App's lab and names the graph it closes**
  — `lab <name>` binds to the unit's `graph <name>: App`, and each
  simulation declares its target: `on app` (the only executable target
  today; sub-graph closure is deferred until a birth recipe exists for
  mid-lineage envelopes). `app` stays pre-bound, and node references in
  mocks and expectations are
  root-relative qualified paths (`sessions.codex`).
- **A scenario is a named slice of that space**: when executed, it becomes
  one deterministic trajectory through the graph, with concrete fixture
  steps, mocked Connector outcomes, and expectations.
- **Mock/Inject/Expect** reads vertically as a Gherkin scenario
  (simulation-semantics.md is the normative spec of this surface).
- **Mocks mirror Connector boundaries**: a mock is written in the same
  handler grammar as a node body, but its factory binds through the same exact
  contract id, roles and family request protocol as the real Driver. It
  replaces only the external interaction, not the Connector's journal
  bookkeeping. The graph in test is the same graph in production.
- **Intermediate assertions** (`expect` on any internal port) test
  transformations without isolating nodes; observation is read-only and
  legal anywhere.
- **Description** is free-form prose, attached as metadata.

The vocabulary not shown in §1.5 stays small — `after <duration>` (a dated
mock emission: "the answer arrives, but too late"), `expect … count == N`
(exact-count assertions), `crash after <record>` / `restart` (kill the
process at a journal boundary and exercise recovery), `origin` (anchor
virtual time to a civil instant for cron/calendar logic). About six words
in total; everything else is the graph grammar itself. The normative
surface is [simulation-semantics.md](docs/spec/simulation-semantics.md).

### 6.3 Experiment — where the Lab meets reality

An `experiment` runs the **real App under an external deployment
profile, with at least one real Connector boundary, in real time**. The
inbound side stays fixture-driven for now — injections still enter at
the source Connector's output — until a partner-side driver/lowering
contract exists. It lives in the Lab beside the simulations and reuses
their scenario grammar:

```rastok
lab checkout {
  simulation payments on app {
    mock bank {                       // in SIMULATION the bank is scripted
      on in { out = { status = "charged" } }
    }
    scenario accepted {
      orders.out = { meta.ext.order_id = "o-1"
                              data = { amount = 120 } }
      expect "posted" ledger.in where data.status == "charged"
    }
  }

  experiment payment_sandbox on app {
    profile = "bank-sandbox"          // exactly one; a constant Rastok text expression —
                                      // no secret, no endpoint in the DSL
    // `bank` is deliberately NOT mocked here : it is the real boundary
    // under test, reached through the profile's sandbox credentials.
    mock notifier {                   // boundaries NOT under test stay mocked
      on in { out = { delivered = true } }
    }
    scenario accepted {
      orders.out = { meta.ext.order_id = "o-2"
                              data = { amount = 120 } }
      expect "charged for real" ledger.in where data.status == "charged"
    }
  }
}
```

An experiment is not merely a slower simulation. It can reveal facts
that the controlled model did not contain: a partner changed a response
shape, a sandbox now applies a different timeout, or an adapter no longer
honors its declared contract. Such a failure is evidence that the
Connector contract, simulation, or both must be updated.

Five things define the contract in
[simulation-semantics.md](docs/spec/simulation-semantics.md) §10:

- It is **evidence, never a universal proof**: evidence about
  *this run against these partners*.
- **`advance` and `origin` are compile errors inside it** — both belong
  to virtual time; an experiment waits in real, bounded time.
- **At least one Connector boundary is real**; the others may stay
  mocked.
- **The inbound side is not real yet**: an injection still writes at
  the source Connector's output, bypassing the real listener, codec,
  auth and transport.
- Closing that gap requires a **partner-side driver contract**, or a
  profile-specific lowering of injections into real requests. Until
  then, an experiment does not cover the real listener, codec,
  authentication, or inbound transport.

### 6.4 Graph coverage and test generation

Because the topology is a static DAG, paths from source to sink are
finitely enumerable. The compiler can produce:

```
$ rastok coverage approved_action_flow

Nodes:    8
Edges:    11
Paths:    4
Branches: 2

Generated test skeletons:
  P1: codex → classify(priv=true) → correlate → gate(ok) → executor
  P2: codex → classify(priv=true) → correlate → gate(deny) → audit
  P3: codex → classify(priv=false) → executor
  P4: operator_ui → correlate(orphan) → audit
```

A skeleton scenario is generated per path and grouped in the App's lab.
The PM/PO fills in the business specifics; the structural coverage is
derived from the graph.

### 6.5 LLM-driven graph derivation

The static DSL plus the simulation harness create the conditions for a
new development workflow:

```
1. A PM writes a specification in natural language and/or Gherkin.
2. The PM or a copilot translates it into the App's Lab: simulations
   (and, later, theorems).
3. An LLM proposes a `graph` that satisfies the Lab.
4. The inner loop is local and fast: compile, then simulate —
   deterministic, seconds per candidate.
5. Once the inner loop is green, experiments run against selected
   sandboxes and partners.
6. Verification adds two distinct operations: bounded challenge
   (`refuted` or `no counterexample within bounds`, never `proved`) and
   the future prover
   (`proved`, `proved under assumptions`, `refuted`, `unknown`).
7. A `refuted` verdict exports a minimal, replayable Scenario. A failed
   experiment returns through the journal and prod-to-fixture bridge.
8. The LLM fixes the graph and repeats the loop.
9. The Lab remains as the locked, living specification.
```
**The LLM generates candidates; the verifier decides.** This is feasible
because the DSL search space is bounded by a typed grammar — the LLM is
constrained to remain within the contract. It is robust because the lab
both specifies and verifies. It is sustainable because the lab becomes
living documentation.

The long trajectory is **predicate- and theorem-driven development**:
the user states what must hold, and searches for a graph such that —
for every admissible input, a safety property stays true; every
execution reaches exactly one terminal state; every `success` output
satisfies its postcondition; under explicit assumptions, a success is
eventually reached; and no arbitrary LLM output crosses a
deterministic guard when it violates policy. These are five *different*
properties — totality, safety, functional correctness, conditional
liveness, termination — and none of them collapses into « always
success »: a refusal or a typed business error is a perfectly correct
terminal state.

Two artifacts anchor the loop in practice. The **counterexample** is a
first-rank Rastok object: a minimal Scenario, replayable, reducible,
and eligible to join the golden traces. The **experiment failure**
closes the loop with reality:

```
real experiment failure → journal → mechanical projection of its
world-half → deterministic fixture → new simulation Scenario → fix →
new proof obligations, new experiment
```
---


## 7. Cross-Cluster Communication

When two Rastok Clusters need to communicate, they do so through a
Connector. There is no transparent cross-cluster routing layer, no
shared global naming scheme, no implicit federation protocol.

```rastok
connector remote_rastok : RastokCluster {
  url = "https://rastok-paris.internal/"
  timeout = duration("30s")
  // in = request to the remote cluster, out = its response (default ports)
}
```

This is an illustrative future Connector, not a specified stdlib
contract. Transport and encoding are deliberately absent. CBOR would
describe an encoding; gRPC would describe a transport/RPC binding.
A future `RastokCluster` contract must pin both where they matter,
without forcing applications to configure a native default explicitly.

A remote Cluster is, from the local Cluster's perspective, just another
external system. The same simulation, mocking, and verification
machinery applies. The reasoning surface ends at the Connector, as it
does for any external boundary.

This restriction is deliberate. Remote invocation has latency, partial
failure, and security boundaries. Rastok keeps those properties visible
instead of presenting a remote Graph as a local call.

---

## 8. Open Source and Trademark

Rastok is released under the **Apache License 2.0**. Anyone may
use, modify, fork, build, run, and redistribute the runtime, the DSL
compiler, the standard library of nodes, and the simulation harness.

The Rastok name and any official logo are governed separately by the
policy in [TRADEMARKS.md](TRADEMARKS.md). Its purpose is to distinguish
official artifacts from third-party distributions and forks while
permitting fair, descriptive use. The rule of thumb is:

> The Rastok trademark is not the toll of the project; it is the
> certificate of origin.

---

## 9. Annex — Inspirations and Positioning

Rastok's model synthesizes ideas that have been developed and
battle-tested by other communities, sometimes for decades.
Acknowledging this lineage is both intellectually honest and
strategically useful: it situates Rastok in a tradition reviewers can
verify. This annex records the genealogy (§9.1–§9.9) and the practical
positioning against adjacent systems (§9.10) — deliberately *after* the
model, so each "what Rastok takes / leaves behind" can be read against
concepts already established.

### 9.1 Synchronous dataflow (Lustre, Esterel, Signal, SCADE)

Created in France in the 1980s by the Lustre, Esterel, and Signal
communities, these languages developed the idea of declaring dataflow
programs as systems of equations over streams, with deterministic
execution and strong formal-analysis tooling. Ansys SCADE carries that
tradition into safety-critical development; its code-generation and
verification tools support qualification or certification up to
standards including DO-178C Level A and IEC 61508 SIL 3.

**What Rastok takes:** the **conviction that a dataflow graph can be both
expressive and mechanically verifiable**. SCADE demonstrates that
graph-shaped programs can participate in demanding assurance workflows.
That is a direction for Rastok, not a certification claim. The `pre(.)`
intuition survives at the
**inter-transaction** boundary: each transaction is one cycle, reading the value the
previous transaction committed.

**What Rastok leaves behind:** almost all of SCADE's *machinery*. The
synchronous hypothesis (a global atomic tick) is incompatible with async
I/O (LLM calls, HTTP, processes) — Rastok is the asynchronous descendant.
The equational / no-reassignment style is gone (Rastok has mutable `var`),
and so is **intra-transaction SSA**: within a transaction the sequence is ordinary
imperative read-modify-write (§2.4). What survives is the *aim*, not the
method — the technical lineage for Rastok's determinism and state is really
Kahn Process Networks (§9.2), I/O Automata (§9.3) and modern keyed-state
(§9.8); SCADE stands as the proof that verifiable dataflow ships in
production.

### 9.2 Kahn Process Networks

Gilles Kahn's 1974 paper described networks of sequential processes
communicating through unbounded FIFO channels, executing in parallel,
with the property that the network output is a deterministic function
of its inputs regardless of the relative speeds of the processes.

**What Rastok takes:** the FIFO-channel semantics on edges and the
parallel execution model.

**What Rastok explicitly does NOT claim**: the network-level determinism of a
closed KPN. A Kahn network is deterministic because a process may only
blocking-read **one** channel at a time; Rastok's multi-input joins fire
on **arrival order across ports** (processing-time timed merges), and
Connector responses arrive with external timing — both are races a pure
KPN excludes (the Brock-Ackerman anomaly). Rastok's contract is
**journal-relative replay determinism** (envelope.md §2.0/§8): the
per-hop journal records each race's outcome; given `(graph, journal)`
the run is fully reproducible; in simulation the fixture plays the
journal's role.

**What Rastok leaves behind:** unbounded channels (Rastok requires
bounded capacities for backpressure), pure-process actors (Rastok adds
state machines), and the assumption of static topology only (Rastok
allows bounded dynamic dispatch).

### 9.3 I/O Automata (Lynch & Tuttle)

Nancy Lynch and Mark Tuttle's work on I/O automata formalized
concurrent systems as parallel compositions of state machines
exchanging actions. It is a useful mathematical lens for Rastok's
intended transition model; the current project does not yet claim a
formal encoding in that model.

**What Rastok takes:** the state-machine-as-fundamental-unit
abstraction, the composition algebra, the proof obligations on local
transitions composing to global properties.

### 9.4 Actor Model (Hewitt, Erlang/OTP, Akka, Orleans)

Carl Hewitt's 1973 actor model and its descendants (Erlang/OTP from
1986 onward, Akka, Orleans) gave the world supervised hierarchies,
isolated state, message-passing concurrency, and fault tolerance
through "let it crash" supervision policies.

**What Rastok takes:** the supervision tree, the actor-as-state-machine
intuition, the partitioned-state-by-identity model (Orleans virtual
actors).

**What Rastok leaves behind:** the unstructured topology — and that is
the defining difference. In a pure actor system any actor may message
any other: addresses are values, passed and discovered at runtime, so
the communication graph is *dynamic and unbounded*. Combined with
arbitrary message interleavings, that general model does not provide a
finite application-level path set by construction.

Rastok constrains communication to a **statically declared DAG**: every
edge is visible in the source, dynamism is bounded (Router / Dispatch
+ inline broadcast `(a, b, …) = x`, §2.6), and all I/O crosses
declared Connectors. Three properties follow that a general actor model
does not guarantee:

- **Finite path surface** — the paths from source to sink are finitely
  enumerable from the topology; per key, transactions are serialized in
  arrival order. Multi-input joins remain timed merges (arrival races),
  so a run is *not* a pure function of its inputs — it is a pure function
  of `(inputs + the journaled arrival orders)`: **journal-relative
  determinism**, reproducible in replay, pinned by the fixture in
  simulation (§9.2, envelope.md §2.0).
- **Testability** — the compiler can enumerate paths and tooling can emit
  a simulation skeleton per path (§6.4); deterministic simulation replays
  a scenario byte-for-byte, with Connectors mocked. Coverage is *measured*,
  not estimated.
- **Verifiability** — a finite, static topology is a finite object a
  model checker or theorem prover can *target* (deadlock-freedom, proper
  completion, delivery guarantees — the future challenge
  and `theorem` levels, §6.1); a dynamic actor topology does not offer
  the same finite target without additional restrictions.

The trade is flexibility for analyzability. Typed actor protocols improve
message-level safety; Rastok additionally fixes the application topology.

### 9.5 Flow-Based Programming

J. Paul Morrison's flow-based programming (formalized in the 1970s,
canonized in his 2010 book) introduced the idea of building programs
as networks of independent components exchanging information packets
through named ports.

**What Rastok takes:** the component-with-named-ports model, the
information-packet (envelope) abstraction, the visual-friendly graph
topology.

**What Rastok leaves behind:** the UI-first emphasis. Rastok is
text-first, version-controlled, code-reviewable.

### 9.6 Functional Core, Imperative Shell (Gary Bernhardt)

Gary Bernhardt's 2012 distinction between a "functional core" (pure,
testable, deterministic) and an "imperative shell" (I/O, mutable,
necessary) crystallized a pattern many practitioners had been using
informally.

**What Rastok takes:** the rigor of separating pure Operators
(functional core) from effectful Connectors (imperative shell).
Rastok's most important architectural invariant — *"Pure logic
belongs in Operators. Effects belong in Connectors."* — is the
direct application of this principle.

Readers coming from functional programming may recognize the
architectural family shared by monadic I/O, free program descriptions,
and algebraic-effect handlers: describe a computation separately from
the interpreter that performs its effects. Rastok does not expose
monads, require a functional host language, or claim a general
algebraic-effects calculus. The kinship is in the boundary discipline:
App execution and Lab simulation interpret the same effect intents
through different handlers.

### 9.7 Tagged frontends and guest languages

Many DSLs and host languages have explored the pattern of
*embedding* a guest language (jq, SQL, regex, shell) as a tagged
block, with the host parser treating the body as opaque. Tcl, Lua
inside Nginx, jq filters embedded in scripts, SQL inside JOOQ —
all variations on the same host/guest pattern.

**What Rastok takes:** the uniform `IDENT!` frontend syntax (`jq! {
... }`, `sql! { ... }`, `json! <value>`) for embedding readers and
guest languages without requiring the host parser to understand them. The guest
language has its own evaluation rules; the DSL turns the body into a
`GuestProgram` for the corresponding `GuestExecutor`.

**What Rastok leaves behind:** the open-ended escape-hatch posture.
Each resolved frontend in Rastok has a fixed framer/compiler/executor
profile (jq, SQL, …) and typed boundaries; the host chooses the syntax
branch from `!`, then the Registry validates the name. A guest that performs external
I/O is Connector-owned rather than Operator-owned. This preserves the
DSL's static-analyzability guarantees while leaving room for pure guests
compiled to Wasm.

### 9.8 Modern dataflow (Beam, Flink, Kafka Streams, Naiad)

Modern stream-processing systems —
Apache Beam, Apache Flink, Kafka Streams, and Frank McSherry's Naiad
/ timely dataflow — operationalized many of these ideas at industrial
scale.

**What Rastok takes:** keyed state and co-partitioned joins (Flink,
Kafka Streams), the `WITHIN` join-window idiom (ksqlDB), the
`TestStream` simulation abstraction (Beam), the logical-time / epoch
coordination (Naiad).

**What Rastok leaves behind:** the batch/analytics focus *and the
watermark machinery*. Beam/Flink reorder by event-time with watermarks
and grace periods; Rastok deliberately matches joins by **arrival
order within the owning logical instance** (processing-time) and bounds them by
**count** (`last`, the static memory guarantee) plus an optional time
window (`within`) — clock-free, deterministic, simulable — accepting
that event-time correctness requires upstream reordering. Those systems
target broad analytical stream processing; Rastok targets operational
graphs whose effects, sessions, and recovery policy are part of the
application model.

### 9.9 Summary of the synthesis

| Tradition                          | Took from it                              |
|------------------------------------|-------------------------------------------|
| Synchronous languages (Lustre/SCADE) | Deterministic dataflow and assurance-grade analysis |
| Kahn Process Networks              | Per-edge FIFO ordering (network-level KPN determinism explicitly not claimed — joins are journaled timed merges) |
| I/O Automata                       | State machine as fundamental unit         |
| Actor Model (Erlang/OTP)           | Supervision, partitioned state            |
| Flow-Based Programming             | Component-with-ports, envelopes           |
| Functional Core, Imperative Shell  | Operator vs Connector distinction         |
| Host/guest language embedding      | jq / SQL guest languages with fixed executors |
| Modern dataflow (Beam, Flink, ksqlDB) | Keyed state, co-partitioned joins, `WITHIN` windows, TestStream |

Rastok's working hypothesis is that this **combination** is useful for
the specific shape of modern stateful and agentic systems.

### 9.10 Practical positioning — adjacent systems

Several categories of system map cleanly onto the Rastok model. The
conceptual model overlaps with their use cases; the current
implementation does not claim operational parity with mature
incumbents.

- **The visual-flow generation** (NiFi, Camel — then Zapier, Make, n8n,
  SmythOS). Rastok's direct ancestry; §1.8 told its story, here
  is the honest scorecard. What NiFi got right — and Rastok keeps:
  provenance as a first-class object, visible bounded queues,
  backpressure as an operational surface, the connector catalogue as
  the unit of reuse. What its *model* does not give — and Rastok is
  built around: a small textual DSL as the **human-authored source of
  truth** (NiFi's Git integrations version canvas snapshots; the
  canvas remains the program), typed envelope contracts between nodes
  (§2.1), state and correlation keyed into the model rather than
  assembled from attributes (§3), deterministic whole-world simulation
  with golden traces (§6), a causal journal built for byte-for-byte
  replay, and one runtime from embedded device to cluster (§4 — where
  NiFi needed the separate MiNiFi sub-project). To be fair in the
  other direction: NiFi today has mature UI, production-hardened
  provenance tooling, clustering operations, and ecosystem breadth
  that Rastok does not claim. The low-code wave (Zapier, Make, n8n) is
  the same paradigm at mainstream scale, and its ceiling is the
  governance wall of §1.8: Rastok does not compete with click-ops for
  the first five workflows — it is where flows go when they outgrow
  click-ops without giving up being seen.

- **Workflow orchestrators** (Airflow, Prefect, Dagster, Temporal).
  Rastok's declared DAG of typed Operators with deterministic
  simulation is a direct evolution of the "DAG of tasks" pattern.
  Compared to Airflow specifically, Rastok intends to offer a dedicated
  declarative DSL, compile-time typing of envelope contracts,
  deterministic simulation as a first-class primitive, and a
  runtime-owned journal. Airflow is a mature Python platform with a
  scheduler, DAG processor, API server, and metadata database; Rastok
  does not yet make a performance or operational-footprint comparison.
  Equivalences: cron triggers map to a `Calendar` (civil time);
  retries and backoff use the `Loop` aspect; XCom-like passing
  becomes typed envelopes on edges; backfill is a replay over the
  event log. The practical gaps to close before claiming full
  Airflow parity: the stdlib `Calendar` (cron + tz, spec'd in stdlib.md),
  documented backfill semantics, and a monitoring UI for live
  logical instances.

- **Agent runtimes and LLM orchestration** (LangGraph, Semantic
  Kernel, ADK). The Operator/Connector model with keyed session
  instances, mockable Connectors, and `on (a, b)` join handlers for
  multi-input correlation covers the common patterns of agentic
  graphs. Rastok adds a constrained typed DSL, specified deterministic
  simulation, and keyed Dispatch as part of the model.

- **Stream gateways and process-backed services** (custom NiFi
  flows, gRPC gateways, Apache Camel routes). The Connector model
  with typed input/output ports and explicit boundary attributes
  (address, codec, protocol, auth) expresses these declaratively,
  with the same Lab contract used by every other Rastok boundary.

**Boundaries.** Rastok is not trying to be every data or automation
system. Some adjacent categories are outside its target; others define
the practical territory Rastok wants to make stricter, simulation-first,
and eventually checkable.

- **Large-scale batch analytics frameworks** (Spark, Hadoop, Apache Beam,
  Flink in its analytics posture). Those systems optimize for petabyte-scale
  batch or streaming analytics. Rastok targets stateful operational graphs:
  agent sessions, process-backed services, integrations, approvals, effects,
  and replayable behavior.
- **Visual automation and flow tools** (NiFi, n8n, Zapier, Camel routes).
  This is close to Rastok's territory. Rastok does not imitate their UI-first
  or low-code model; it takes the underlying need — integration flows,
  stateful routing, retries, effects, and operational visibility — and moves
  it into a typed, text-first, simulation-first graph model. The source is
  reviewable, generatable, testable, replayable, and built so that future
  versions can prove selected graph properties when the graph stays within
  the decidable fragment.
- **Hard real-time and safety-critical systems** (SCADE, Ada/SPARK,
  certified Lustre toolchains). Those targets have certified toolchains
  (DO-178C, IEC 61508). Rastok borrows their taste for static structure,
  variants, typing and analyzability, but targets cloud-native asynchronous
  systems rather than certified avionics or hard real-time control.

---

## 10. Status

Rastok is at an early experimental stage. It is not production-ready.
What exists today is the conceptual model, the normative specification
under review, a minimal Rust workspace, and reference graphs and
simulations designed to become executable evidence as the runtime
lands. The current evidence gates are listed in
[ROADMAP.md](ROADMAP.md).

As of M1.6 H5.1, the one-worker reference executes canonical keyed/lifetime
instances, bounded Closing, nested Graph composition, and instance-local
bounded joins from source through package validation and public Lab scenarios.
The closure audit also proves that moving a stable virtual shard between
physical Host/Worker assignments changes neither semantic identity, causal
journal bytes, nor replay input. This is milestone evidence, not a
production-readiness or cluster-runtime claim; distributed leases, fencing,
recovery, and scheduling remain later physical implementations of the same
logical addresses. Process and File now execute as two real outbound Driver
families through one exact-id/role registry and one VM boundary; the Lab
composes both through the same binding and protocol. Static Driver selection is
implemented. A data-only capability manifest proves the future discovery seam,
but no plugin loader or native ABI is claimed. Four exact HTTP/WebSocket
contracts now cross source, checked v12 plan, Registry-free validation and
family-owned Lab binding with codec-selected ports, lossless protocol metadata
and typed terminal unions. H3 pins one bounded private hosted
pump, Hyper/Rustls for HTTP/1.1-2, quiche/BoringSSL for HTTP/3 and Tungstenite
for WebSocket framing. It proves all three HTTP engines coexist, compares
h3/Quinn on real UDP and proves bounded WebSocket framing plus HTTP/1.1
Upgrade. H4 now executes HttpClient over all three versions, including exact
pre-open request sealing, visibility-safe fallback, bounded bodies, lossless
response observations, generic Call conformance and outbound backpressure.
H4.1 adds the open `media_type` scalar, typed standard/custom request-header
separation, typed response metadata and strict project-wide OpenAPI type
resolution without changing the runtime's one type algebra. H5's generic
inbound Call seam proves bounded commit-before-accept at the exact source
instance and close-once typed `l:reply`. H5.1 attaches that seam to a real
bounded Hyper listener: HTTP/1.1 and HTTP/2 ack requests keep body memory
charged through admission and cannot expose 202 before the causal root
commits. HTTP/3 server binding, graph-driven responses and independent server
interop remain H5 work; H6 still owns live WebSocket sessions and both
Extended CONNECT adapters.

This white paper is designed to outlive individual releases. The DSL
grammar, runtime traits, and feature set will evolve; the conceptual
model laid out here is the contract Rastok intends to keep.

---

## 11. Maxims

A short list of phrases, useful when in doubt. They are mnemonics, not
norms — each compresses a rule whose exact contract lives in
[docs/spec/](docs/spec/); when a maxim and a spec disagree, the spec
wins.

1. **Pure logic belongs in Operators. Effects belong in Connectors.**
2. **A Node fires on an input event and emits zero or more envelopes
   atomically. It is not a tuple-to-tuple function.**
3. **Topology is static. Dispatch is dynamic. The dynamism is bounded
   by the topology.**
4. **Every mutable cell has one Node owner. Key selection addresses a
   logical instance; lifetime closes it; lexical scope exposes immutable
   context. Do not fuse the three.**
5. **Cycles do not appear in the graph. They appear in state, in
   events, and in sequence.**
6. **Operators do not read a hidden clock. Sequence is journaled;
   wall-clock observations enter through Clock or Calendar Connectors.**
7. **The runtime is asynchronous. The simulation is deterministic.
   Same world-half in, byte-identical causal cores out — the two modes
   agree journal-relative.**
8. **The trademark is the certificate of origin, not the toll of the
   project.**
9. **Two clusters speak through a Connector, like any other external
   system. No magic, no implicit federation.**
10. **The simulation is the specification. The graph is the
    implementation. The toolchain checks the graph satisfies the
    declared scenarios.**
11. **State has three durability scopes: none (ephemeral), local
    (survives the process), and global (survives a Host). A Node's
    effective scope is its explicit value, otherwise its nearest
    enclosing Graph, otherwise the App, otherwise none.**
12. **The same App runs from embedded (single device, no
    persistence) to clustered (multi-Host SaaS). The mode is a
    deployment choice, not a code choice.**
13. **An envelope is `meta + data`. `meta` is three zones of typed
    scalars (or scalar arrays): `meta.sys` read-only and
    runtime-fresh, `meta.app` and `meta.ext` writable. `data` is
    the typed payload. Envelopes are immutable in flight; Operators
    emit new ones. Single input inherits `meta` additively; a join
    merges by value — common fields survive, conflicts drop, and
    explicit writes win (D-2026-08-10-2).**
14. **A logical Node instance starts lazily, runs mailbox transactions,
    enters bounded Closing by `stop` or lifetime expiry, runs `on stop` last,
    and may later be recreated at the same address as a fresh generation.**
15. **Two keywords, scope by position: `val` for immutable context and
    `var` for mutable Node-local state with read-modify-write per transaction.
    Graph `val`s are lexical; no child inherits an ancestor `var`.**
16. **Ports may be inferred from handlers, while `input` / `output`
    pin or document constraints. Every validated port has one effective
    logical type. `in` / `out` / `err` are conventions, not categories;
    an Operator has no implicit `err`. In the simple shape, explicit
    `err: E` makes `I -> Result<R,E>` while engine faults abort and reach
    supervision rather than becoming envelopes.
    Connector contracts declare fixed or parametric port rules. For
    each invocation, a Connector conventionally uses `exit` for its tagged
    terminal outcome. For Process, `err` is stderr and `exit` carries
    `ProcessExit`, including the original typed error cases. One completion
    carries `out* + err* + exactly one exit`; `stop` independently closes the
    owning Node instance. Engine faults and panics are supervision,
    never envelopes.**
17. **The `data` payload is statically typed by default; `variant` is
    the only dynamic type, narrowed before use. The type tree is
    scalar(parameters+logical) / struct / map / array / enum / union,
    anchored on Arrow. Supported JSON Schema, Avro and Arrow schemas —
    including unions — project into this one algebra without lossy
    fallback. `table` is the relational view of
    `array<struct>`, not a type. Numbers are lossless until narrowed.**
18. **A handler is a transaction guard. `on (a, b)` is a join correlated by
    its owning Operator instance; content correlation is a θ-join and demands
    a bound (`last` for memory, optionally `within` for staleness).
    OR across ports is separate handlers, not `||`.**

19. **The Lab is the App's twin, never woven into the deployed graph:
    simulations close graphs, experiments exercise real boundaries,
    challenges search bounded spaces, and future theorems may establish
    proofs.**
20. **A Connector contract names semantics; a Driver supplies one physical
    implementation. Exact ids and roles meet once at binding. Static builds and
    future plugin hosts must feed the same registry, while the VM remains
    ignorant of concrete Connector families.**

These maxims are not arbitrary slogans. Each one resolves a specific
architectural ambiguity, and contributors are expected to know them.
