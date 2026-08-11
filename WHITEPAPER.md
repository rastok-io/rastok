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

> *Rastok is a graph of stateful state machines, keyed by named
> partitions, communicating through immutable envelopes over a strictly
> acyclic topology, specifiable by executable simulations.*

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
| Long-lived keyed state | Session state is scattered across workers, caches, and correlation tables. | Named partitions index state, serialize work per key, and expose lifetime. |
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

> Rastok = a typed DAG of state machines, keyed by named partitions,
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
3. **Named partitions define keying dimensions for stateful Nodes**
   (§3). A partition does not represent global graph state. It tells
   the runtime how to index a Node's state. Processing is serialized
   per key and parallel across keys, which supports local reasoning and
   sharding (§4).
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
  partition expense { key = meta.ext.request_id ; lifetime = duration("72h") }
  output rejection

  connector requests   : HttpServer { path = "/requests" }    // two independent sources
  connector decisions  : HttpServer { path = "/decisions" }
  connector accounting : HttpClient {
    url             = "https://accounting.internal/entries"
    effect          = irreversible
    behavior        = deterministic
    idempotency_key = meta.ext.request_id
  }

  operator decide per expense {                    // state keyed by the expense
    output approval
    output rejection

    on (request, decision)
      where decision.data.approved
      within duration("48h") {
      approval = {
        meta.ext.request_id = request.meta.ext.request_id   // a join picks explicitly; common fields merge (CommonOrAbsent)
        data = { request_id = expense.key,
                        amount     = request.data.amount }
      }
    }

    on (request, decision)
      where !decision.data.approved
      within duration("48h") {
      rejection = {
        meta.ext.request_id = request.meta.ext.request_id
        data = { request_id = expense.key,
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
  // decide.err is deliberately not wired: errors bubble to `app.err`,
  // the root error sink — observable in simulation, wirable in production.
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

      expect "expiry is typed" app.err where err.kind == "JoinExpired"
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

- **Stateful correlation is the model, not plumbing.** `per expense`
  keys the operator's state; the join `on (request, decision)` matches
  the two legs of the *same* expense automatically, whatever the arrival
  order.
- **Business outcomes and technical errors remain distinct.** The two
  guarded handlers emit `approval` or `rejection`. Missing input is not a
  business refusal: after 48 hours it becomes the technical
  `JoinExpired` error on `err`.
- **The simulation drives the world.** The *called* system
  (`accounting`) is mocked; the *sources* are driven by injection;
  `advance duration("48h")` moves virtual time deterministically. Nothing here
  touches a network or a clock.
- **The lab is the App's twin.** `lab expense_approval` binds to the
  homonymous `: App` graph (§2.2, §6) and is never deployed; its
  simulation names the graph it closes (`on app` — `app` is pre-bound,
  hence `app.err` above), and node references resolve in the unit's
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
graph sessions per session {          // declaration = instantiation; `per` = one instance per key
  max_instances = 1000                // Dispatch knobs are ordinary body attributes
  spawn_policy  = lazy

  partition session { key = meta.app.session_key ; lifetime = duration("2h") }
  val cwd = "/workspace/" + session.key            // per-instance constant (CEL)

  connector codex : Process {
    command = "codex"
    args    = ["app-server"]
    mode    = service                              // resident child process, one per session
    cwd     = cwd
    codec   = jsonl
    effect  = idempotent
    behavior = probabilistic
    timeout = duration("120s")                                 // per-call budget
    restart = on_crash { backoff = "1s..30s", max_attempts = 5 }
  }

  in        -> codex.in { capacity = 1024, backpressure = block }
  codex.out -> out
  // codex termination = a typed terminal error (ProcessExited) on codex.err,
  // bubbling to app.err; every boundary crossing is journaled, so
  // per-session cost is an aggregate over the journal, not a guess.
}

graph agent_gateway : App {
  connector teams_in  : HttpServer { path = "/api/messages" }
  connector teams_out : HttpClient {
    url             = "https://smba.trafficmanager.net/"
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
      mock sessions.codex {             // a qualified path into the per-key sub-graph
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

The rule this example makes visible: **declaring is placing, `per` is
cardinality, wiring is naming.** A `graph` is a Node like any other —
`sessions` is declared once and wired by name. Because it has
`per session`, the runtime materializes one graph instance for each
distinct session key. Every child Node belongs to that graph instance.
A child with its own `per` adds that keying dimension; a child without
one has one instance inside its parent Graph instance. The
`max_instances` and `spawn_policy` attributes control when those
per-key Graph instances are created and how many may coexist.

Beyond the mechanics, note the operational posture: a thousand
concurrent sessions are one declaration and one knob — and one journal.
Per-session cost (every LLM and API crossing is an OUTBOUND record) is
an aggregation over the journal, not an instrumentation project; a
dying `codex` is a typed terminal error with a declared restart policy,
not a 3 a.m. mystery; and the whole gateway is simulated — process,
clock and Teams included — before it ever touches production.

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

Rastok recognizes exactly **three families of node**, sharing the same
underlying structure:

```
Node
├── Operator   — pure, event-driven computation inside the graph
├── Connector  — reads from or writes to the external environment
└── Graph      — composite Node, wired by name like any other Node
```

An Operator reacts to envelope arrivals on its inputs, optionally reads
and updates state keyed by a partition, and emits zero or more
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
Arrow/Polars and SQL-style tables. It keeps the important distinction between
value-world coercions, boundary codecs, and lazy parsing so pure routing does
not pay parse cost. The exact type tree and grammar are normative in
[docs/spec/grammar.md](docs/spec/grammar.md); the remaining `types.md`
extraction is tracked in [docs/spec/README.md](docs/spec/README.md).

### 2.2 The shared Node shape

Every Node, regardless of family, shares one declarative skeleton:

```
<family> <name> [: <annotation>] [per <partition>] { <body> }

family     : operator | connector | graph        (a graph instantiates by DECLARATION
                                                   and wires by name ; the `: App` role
                                                   marks the deployment entry graph ;
                                                   mock, simulation-only, mirrors the
                                                   same skeleton as a stand-in)
annotation : trait/role + aspects — optional on operator/graph (implicit canonical
             types `Operator` / `Graph`), MANDATORY on connector (a boundary is explicit)
```

The port-declaration keywords are **`input`** and **`output`**; the names
**`in`**, **`out`** and **`err`** are *default ports*, pre-bound and
non-reserved (usable without declaration, redefinable). `in` is the
default input; `out` and `err` are default outputs. This mirrors Unix
stdio exactly — a `Process` Connector's `in` / `out` / `err` *are* its
stdin / stdout / stderr.

```rastok
operator <name> [: <trait + aspects>] [per <partition>] {
  input  <port> [: <type>]   // type OPTIONAL — inferred unless pinned ; in/out/err exist by default
  output <port> [: <type>]   // a single-in/out node just uses the defaults

  var <field>: <type> [= <default>]    // optional state fields

  on start            { ... }          // partition starts (lazy, first key)
  on <trigger> [where …] [having …] [within …] [last …] { ... }   // see §2.3
  on stop             { ... }          // partition ends (lifetime expired)
}
```

A Connector adds boundary attributes describing its bridge to the
outside world; built-ins do not re-declare their ports (their type
defines them):

```
connector <name> : <NativeType> [per <partition>] {
  <attr> = <value>           // e.g., url = "...", codec = json ; Connectors declare NO ports
  on <trigger> { ... }       // (the type + codec define in/out/err)
}
```

A sub-graph is a Node too: **its declaration is its instance**. It is
keyed by its own `per <partition>` and wired by name. Dispatch knobs
are body attributes:

```
graph <name> [: App | <aspects>] [per <partition>] {   // per → one instance per key (Dispatch, §2.6)
  max_instances = 1000                 // optional Dispatch knobs (body attributes)
  val <field> = "..." + <partition>.key + "..."   // per-instance graph-level constants (CEL)
  ...                                  // operators / connectors / sub-graphs / wiring
}
```

Three categories of section:

- **Declarative interface** (`input`, `output`, `var`, `per`). **Ports
  are implicit**: `on <name>` declares an input, an assignment
  `<name> = …` declares an output, and `in`/`out`/`err` exist by
  default — so a Node usually declares no ports at all. `input` /
  `output` are written only to *pin* a type/schema, or to document a
  node with no handler (a saga) or a graph's public interface. `var`
  holds state; `per` keys it to a partition.
- **Active behavior** — the `on <trigger>` handlers (§2.3).
  - *Implicit yield:* for an Operator with a single output, an
    unassigned trailing expression (e.g. `jq { ... }`) yields to
    `out`. Multiple ports are written explicitly; **atomic
    multi-emit** uses tuple assignment `(out, err) = (e1, e2)`. A
    handler may also emit nothing (pure state update).
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
| `on (a, b, …)` | envelopes are present on all named ports (a **join**), zipped by arrival order within the partition |

OR across ports is expressed as **separate handlers** (`on a { }` /
`on b { }`) — there is no `||` in a trigger.

**Refinements.** `where <pred>` is a predicate over the matched
envelopes' content; `having count(p) >= k` is a quorum (k-of-n).

**Join key = partition key.** For an Operator `per K`, every envelope
on every input must yield `K` (that is how it routes to the right
partition instance) — so a join correlates by `K` for free; `by
<expr>` overrides the key when needed. The §1.5 approval graph is
exactly this: `on (request, decision)` matches the two legs of the
same expense because both carry the `expense` key.

**Joins are bounded — and the bound is behavior.** A presence join
(`on (a, b)`) zips by arrival order within the partition and is
bounded by edge backpressure. A join whose `where` *correlates the
content* of several ports is a θ-join and must declare an explicit
bound: `last N` (count — the static memory guarantee, clock-free)
and/or `within D` (staleness — ksqlDB's `WITHIN`). A candidate that
leaves the join unmatched exits as the typed error **`JoinExpired`** on
`err` — routed and observable like any other error, never a silent
drop, never a background timer. The exact tier classification, bound
composition and lazy-eviction rules are normative in
[execution-model.md](docs/spec/execution-model.md) §7 and
[grammar.md](docs/spec/grammar.md).

**The typed `err` channel.** A transaction may emit on `err`; its error
set is *statically derivable* (explicit `err = …` writes, fallible
narrowings on inputs, a Connector's declared failure modes — the `E`
of `f: in → Result<out, err>`). Genuine panics (bugs) are supervision
faults, never an envelope — this preserves static analysis and
deterministic simulation.

**Guest-language blocks in multi-input handlers.** In a single-input handler a
jq block's `.` is the input envelope; in a join there is no implicit
`.` — reference envelopes by port name (`a.data.x`, plain CEL) and scope
a jq block to one input when needed (`a | jq { … }`). See §2.5 for the
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

**Lifecycle.** `on start` fires when a partition key is first seen;
`on stop` when its `lifetime` expires. Inside any handler the **`stop`**
action ends the current partition instance after the handler completes
(`stop <partition>` to target one of several bound partitions). To
reset a `var` without ending the partition, just assign it (`count =
0`). `on stop` is the time-driven counterpart of the event-driven
`stop` action.

**No `on tick`.** All time-based behavior comes from a `Clock`
wired to an input — time enters the graph as envelopes (§5):

```rastok
operator stats per session {
  input  tick : timestamp                // a Clock tick's payload IS the timestamp
  var count: int = 0

  on event { count = count + 1 }         // scalar state update : plain CEL

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
the `order` partition key):

```rastok
operator settle per order {
  on (invoice, payment) {
    out = {
      meta.app.invoice_id = invoice.meta.app.invoice_id   // a join picks explicitly; common fields merge (see below)
      data = { order_id = order.key,     // `order` = the partition → its key
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

### 2.4 State and partition reads

State is **read-modify-write, imperatively**, within a transaction. A
persistent field `var x: T = default` is read by its bare name (`x`)
— or `$x` inside a `jq { … }` block (jq's own syntax for bound
variables). At the *start* of a transaction, `x` holds the value committed
by the previous transaction (or the default if uninitialized); assignments
mutate it **in place**, so a later read in the same transaction sees the new
value, exactly like an imperative function. The transaction as a whole still
commits atomically — state delta + emissions + audit — at its end.

```rastok
operator counter per session {
  var count: int = 0

  on msg {
    count = count + 1                        // mutate ; reads below see the new value
    out = { meta.app.count = count }
  }
}
```

**Scalar computation uses CEL.** The
[Common Expression Language](https://cel.dev) is a small, typed,
terminating expression language designed for safe embedding. Rastok
uses it because expressions can be checked, bounded, and evaluated
without granting ambient I/O. The right-hand side of an assignment is
a CEL expression (§2.5): `count = count + 1`,
`x = x * 2 + data.n`, `label = meta.app.region + "-" + tenant.key`. CEL is
total — no loops, no recursion, no side effects — so every state
expression is statically checkable. Structural *reshaping* of documents
(building nested JSON, filtering arrays) belongs to guest-language blocks
`jq { … }` / `sql { … }` (§2.5); scalar state never needs them. CEL
positions do not admit effectful functions such as `now()`,
environment lookup, network access, or filesystem reads.

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

**Read-only cross-partition access.** An Operator bound to one
partition may read another partition's state by nominal lookup. It
cannot write that state. The runtime
extracts the other partition's key from the current envelope and
returns the requested var:

```rastok
operator session_metric per session {
  var count: int = 0

  on msg {
    count = count + 1

    out = {
      meta.app.tenant_plan = tenant.?plan_tier.orValue("free")   // cross-partition read, with default
      meta.app.session_id = session.key                // current partition's key
      meta.app.trigger_id = meta.sys.id                // current envelope's derived id
      meta.app.count      = count                      // my own state (new value, imperative)
    }
  }
}
```

The compiler resolves `<partition>.<field>` against the
partitions declared in the graph (see §3).

**Two rules for cross-partition reads**:

1. **Uniqueness (compile-time)** — for each `(partition, field)`
   pair, at most one Operator in the graph may declare that
   var. Two Operators `per tenant` both declaring `var plan_tier`
   is a build error: the cross-read would be ambiguous. Rename
   one, or merge the Operators.
2. **Absence handling (compile-time + runtime)** — a
   cross-partition read produces `T?` (an optional). If upstream
   paths to the reading Operator don't all guarantee the
   partition's key is present in `meta.app`, the compiler emits a
   warning. Absence follows the CEL absent-field grid
   ([grammar.md](docs/spec/grammar.md) §References; the old « missing ≡ null » rule
   survives only inside `jq { }` blocks). The dev handles legitimate
   absence explicitly — `.?` + `.orValue()` in CEL, `//` in jq:

   ```
   val plan = tenant.?plan_tier.orValue("free")   // CEL: absence is explicit
   val cap  = jq { .data.max_tokens // 1000 }     // inside jq: a missing DOCUMENT field is null
   ```

Writes are not allowed across partitions — only the Operator
declared `per <partition>` may write its state. Cross-partition
access is read-only.

#### 2.4.1 Values and state: `val` / `var`

Rastok has only two keywords for named values, following the Scala
/ Kotlin / Swift convention: **`val` for immutable, `var` for
mutable**. The scope is determined by *where* the declaration sits,
not by the keyword.

| Keyword | Where | Mutable? | Lifetime |
|---|---|---|---|
| `val` | App-level | no | App run (global constants) |
| `val` | Graph-level | no | Graph instance — per partition key when the graph is `per …` (derive from `<partition>.key`, plain CEL) |
| `val` | Operator-level | no | App run (Operator-scoped constants) |
| `val` | Inside a handler | no | One transaction (handler-local binding) |
| `var` | Operator-level (with or without `per`) | yes (read-modify-write per transaction) | Partition slot (or graph-scoped) |
| `var` | Graph-level (shared across operators) | — | **deferred** — depends on a complete per-key concurrency contract |

```rastok
graph my_app : App {
  val DAILY_QUOTA   = 1000               // App-level constant
  val SLACK_CHANNEL = "#alerts"
}

operator pricing per tenant {
  val TIERS = { "free": 0, "pro": 99, "enterprise": 999 }   // Operator-level constant (a map: `:` keys)
  var subscribed_tier: text = "free"                   // partition state

  on upgrade {
    val tier = data.new_tier                           // handler-local binding (val = DRY, used twice)
    subscribed_tier = tier
    out = { meta.app.price = TIERS[tier] }    // all of it plain CEL
  }
}
```

A graph declared `per <partition>` also holds **graph-level `val`s** — per-instance
constants derived from `<partition>.key` in plain CEL (e.g. a session's home dir / cwd),
referenced by name by the nodes inside it (see §3 and the
[grammar](docs/spec/grammar.md)). Mutable per-key state stays in an operator
`var per …`; a *shared* graph-level `var` across operators is deferred (it
depends on a complete per-key concurrency contract).

Reading rules:
- **In bare DSL** (envelope literals, assignments, conditions):
  `val` and `var` are both read by their bare name (`X`).
- **Inside `jq { … }` / `sql { … }` blocks**: the guest language's
  own convention applies — jq uses `$X` for bound variables,
  SQL uses `?` parameters or named bindings depending on dialect.
- **Writing `var X`**: `X = <expr>` (assignment, bare).
- Both bare `X` (in DSL) and `$X` (inside jq) read the **same**
  previously committed value. They are the same operation
  expressed in the respective grammars.

Tunable runtime knobs (e.g., a feature flag that an admin can flip
without redeploying) are not a separate concept — they are just a
graph-scoped Operator with a `var` and an admin control port:

```rastok
operator settings {
  var rate_limit: int = 100

  on admin { rate_limit = data.?rate_limit.orValue(rate_limit) }  // absent field → keep current
}

operator stamp per tenant {
  on req {
    out = { meta.app.rate_limit = settings.rate_limit }  // read the live knob
  }
}
```

The admin pushes envelopes to `settings.admin` (via the Cloud UI
or an HTTP endpoint) and the change propagates without redeployment —
an Operator without `per` holds a single graph-scoped cell, and other
nodes read it by name (`settings.rate_limit`) under the same rules as a
cross-partition read: one writer, read-only for everyone else.

### 2.5 Computation: CEL, guest languages, helpers, and primitives

#### 2.5.1 The core expression language and its guests

The DSL itself is deliberately **declarative** — nodes, ports, handlers,
wiring. Scalar computation uses one integrated core expression language,
CEL. Structural or specialized computation may use registered guest
languages with fixed frontends/executors and typed boundaries. jq is
the initial document guest; SQL is the planned relational guest.
Additional guests are admitted only through the same contract.

**CEL — the scalar core.** Every *scalar position* in the DSL — a
`where` guard, a partition key, a Router selector, a state expression,
an attribute value, a simulation expectation — is one expression in the
[Common Expression Language](https://cel.dev):

```rastok
on payment where data.amount > 100 && meta.app.region == "eu" { … }
count = count + 1
key   = meta.app.session_id
```

CEL is *total*: it always terminates, has no side effects, and cannot
express loops. That is the point — every scalar expression in a graph
can be type-checked, costed, and simulated without running anything.
Absence is explicit: `data.?discount.orValue(0)` reads a
possibly-missing field with a default; a *bare* read of an absent field
routes the envelope to `err`, never a silent null.

**`jq { … }` — document reshaping.** When the job is structural — build
a nested JSON body, filter and restructure a collection — a tagged jq
block hands its body to the jq guest:

```rastok
data = jq {
  .data | { url:    "https://api.weather.example/v1/current",
            method: "GET",
            query:  { city: .city, country: (.country // "FR") } }
}
```

Inside the block, jq's own conventions apply: `.` is the input
envelope, DSL bindings are read as `$x`, and a missing field is `null`
(the jq tradition — it survives *only* inside jq blocks).

**`sql { … }` — the planned relational view.** Data of type
`array<struct>` can be treated as a `table` (a view, not a type) and
queried with SQL — aggregations, group-bys, top-k — instead of
simulating relational algebra in jq.

The Rastok DSL is the host language; CEL is its integrated core expression
language. The host parser never interprets a guest-language block's body: it
delimits it, hands it to the named guest frontend, and type-checks the
boundary. The build produces a versioned `GuestProgram`, executed by a pinned
`GuestExecutor`. Three rules keep this analyzable: each tag has a **fixed
frontend and executor** — no unregistered host-language code can be smuggled
through a block (§9.7); every block has **typed input and output**; and **no
position accepts two guest languages** — where CEL is expected, only CEL
parses. jq is the initial specified pure guest; SQL is planned and must
satisfy the same six-clause contract in
[stdlib.md](docs/spec/stdlib.md) (pure determinism, resource budgets, typed
errors — a guest program may never hang the graph).

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
  the outside world through native code (HTTP libraries, process
  spawning, database drivers, file I/O). A new Connector requires a
  Rust (or Wasm) module registered with the runtime; it cannot be
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
| Dispatch   | Per-key sub-graph instance           | The sub-graph is statically declared     |

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

Here, `select` is CEL. The runtime value chooses a route, while the
possible ports remain visible to the compiler. A list of port names
selects several outputs.

Dispatch is the per-key instantiation of a declared sub-graph:

```rastok
partition session {
  key = meta.app.session_id
  lifetime = duration("2h")
}

graph conversation per session {
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

- **In state and sequence.** A `per`-keyed Operator accumulates across
  arrivals (§2.4, §3) — what a `while` loop does with a mutable variable,
  a partition does across envelopes.
- **In the Connector** (§2.7). External feedback loops — a user replying
  turn after turn, a retried HTTP call — stay outside the graph; each
  round trip re-enters as a fresh arrival.
- **In the `Loop` aspect.** Bounded self-iteration — "refine until the
  score passes, at most 10 times" — is declared by annotating a node
  `: Loop` and configuring its well-founded measure:

  ```rastok
  graph refine: Loop {
    Loop.variant = measure {
      initial = 10
      next = value - 1
      until = value <= 0
    }
    …
  }
  ```

  `Loop.variant` is an attribute of the `Loop` aspect, not a `var` in
  Operator state and not the dynamic data type named `variant`. The
  fields on the right are CEL expressions; `value` is the current
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

**Lifecycle-bound resources carry an implicit partition.** A Connector
that owns a resource with a lifecycle — a `Process` child, a `WebSocket`
connection, a database transaction — has an **implicit partition keyed by
the resource instance** (a runtime handle, e.g. a per-spawn `run_id`),
whose lifetime *is* the resource's. Composed under an explicit `per K`,
this is simply a flat composite key `(K, run_id)` — never a hierarchy
(§3). When the resource ends, the Connector emits a **terminal envelope**
on `err`: a typed error, terminal *by kind*, whose payload carries the
outcome — `ProcessExited { exit_code, signal, run_id }` for a `Process`,
the close code / reason for a `WebSocket`, the final status for a
transaction. The outcome lives in the typed error payload; `meta.sys`
stays closed (its five fields never grow per-Connector). `out`
stays the data stream; a graph that reacts to termination wires
`process.err -> …`. `restart = on_crash { … }` (a Connector attribute)
spawns a fresh instance — a new `run_id`, a new partition instance — with
no cascade to any enclosing partition.

---

## 3. Partitions and Keys

Stateful Operators bind their state to a **partition**. A partition
is two things: a **key** extracted from the envelope, and a
**lifetime** that tells the runtime when to clean up.

```rastok
partition session {
  key      = meta.app.session_id
  lifetime = duration("2h")
}

partition tenant {
  key      = meta.app.tenant_id
  lifetime = persistent
}
```

The key is a CEL expression. The lifetime is a DSL policy:
`persistent` is a structural DSL value, not a CEL expression or a
clock read. That is the entire shape of a partition: a name, a key,
and a lifetime. **No parent declaration. No hierarchy. No automatic
cascade.** This is deliberate — the same flat keyed-state
pattern that Kafka Streams, Flink, and Orleans have proven at scale
for a decade.

> **Note on terminology** — at the DSL level, "partition" is a
> *named dimension* declared by the developer. At the runtime level,
> the keys of a partition are sliced across multiple Hosts; each
> slice is called a **shard**. See §4.3.

### 3.1 Binding state with `per`

An Operator binds its state to a partition using `per`:

```rastok
operator counter per session {
  var count: int = 0

  on msg {
    count = count + 1
    out = { meta.app.count = count }
  }
}
```

The assignment mutates in place; the emit reads the **new** value
(imperative intra-transaction semantics, see §2.4).

The runtime, at each transaction, extracts the session key from the
envelope (via `session.key`) and indexes the Operator's state by
that key. Two envelopes with different session keys see independent
state; two envelopes with the same session key share state and are
serialized in arrival order.

**Multi-key partitions** use a tuple:

```rastok
operator metric per tenant, session {
  // state indexed by (tenant_key, session_key) tuple
}
```

This is a **flat composite key**, *not* a hierarchy: there is no
`tenant ⊃ session` nesting, no parent, no cascade — just one state cell
per distinct `(tenant_key, session_key)`. A lifecycle-bound Connector
(§2.7) adds its own **implicit dimension** the same way: a `Process` per
session is keyed `(session_key, run_id)`, the `run_id` being a runtime
handle, not an envelope field — still flat, still no cascade. Resource
cleanup when a key idles out happens via teardown of the per-key Dispatch
instance (§2.6), never by partition cascade.

**Without `per`**, an Operator has one state cell inside each instance
of its containing Graph. This is not a free global variable: the state
still belongs to that Operator, other Nodes can only read it through
the nominal read-only rule, and a per-key parent Graph gives each parent
instance its own cell.

Partitions are declared explicitly, then referenced by name:

```rastok
partition cart {
  key = meta.app.user_id
  lifetime = duration("30m")
}

operator cart_aggregator per cart {
  ...
}
```

A partition may also be declared **inline and anonymous** on a node's `per`,
for the common case where a single node owns it:

```rastok
operator cart_aggregator per ( key = meta.app.user_id, lifetime = duration("30m") ) { … }
graph    session_graph   per { key = meta.app.session_key ; lifetime = duration("2h") } { … }
```

The inline form has **no name** — so it supports no cross-partition reads and no
`<partition>.key` references (plain CEL — the `${…}` interpolation is gone, [grammar.md](docs/spec/grammar.md) §References). When a partition is shared by several nodes,
or a graph needs to interpolate its key, declare it as a named block and
reference it by name. Composite keys (`per a, b`) are named references only.

### 3.2 Cross-partition reads

A useful long form to keep in mind: a « partition » is always **the
partition of a node's state** — the `partition` declaration is the
shared slicing *scheme* (key + lifetime), while the sliced state
belongs to one node. Sharing is therefore asymmetric by construction:
one writer, read-only for everyone else.

An Operator bound to one partition may **read** another partition's
state by nominal lookup:

```rastok
operator session_metric per session {
  var tier_cached: text?

  on msg {
    tier_cached = tenant.?plan_tier.orValue("free")   // plain CEL, absence explicit
    // ...
  }
}
```

The runtime extracts the `tenant` key from the envelope (via
`tenant.key`), finds the state of whatever Operator declared
`var plan_tier` under `per tenant`, and returns it. The dev
references `<partition>.<field>` by name.

The reserved name `<partition>.key` always yields the current
envelope's value for that partition's key (e.g., `tenant.key`
returns the string from `meta.app.tenant_id`). State fields named
`key` are therefore disallowed in Operators bound to a partition.

**Writes are not allowed across partitions.** Only a Node bound to
a partition may write that partition's state. This prevents
action-at-a-distance and keeps data flow visible in the graph
topology.

### 3.3 The envelope is not a partition

Each envelope carries runtime-managed metadata in the **`meta.sys`**
namespace. The canonical field list and terminal-envelope additions are frozen
in [docs/spec/envelope.md](docs/spec/envelope.md). These fields are read
directly:

```
meta.sys.id          // this envelope's derived identity (blake3, replay-stable)
meta.sys.parent_id   // immediate ancestor — the lineage is a linked list
meta.sys.origin_id   // lineage root = OTel trace id
```

Per-hop facts — arrival order (`seq`), commit time (`committed_at`),
the partition key at each keyed node, join contributions — are **NOT**
envelope fields; they live in the per-hop **journal**
([docs/spec/delivery-semantics.md](docs/spec/delivery-semantics.md)). The
envelope carries identity; the journal carries events.

The envelope is **the message currently being processed**, not a
partition. There is no `per envelope` — it would create a state
cell that is immediately discarded. **`meta.sys` is read-only** to
Operators (absolutely, including in simulation): transport
request/response correlation is set by the runtime when crossing a
Connector; business correlation is a join `on (a, b)`; an echo token
an external system imposes is an `app` field. Writable metadata lives
in `meta.app` (the application's zone) or `meta.ext` (the external
boundaries' zone).

### 3.4 Lifetime and termination

A partition instance can end in one of two ways:

**1. Time-driven** — the `lifetime` declared on the partition expires.
A small vocabulary covers the standard cases:

```rastok
lifetime = duration("2h")                  // idle-ttl, the dominant case
lifetime = persistent                      // alive until explicitly revoked
lifetime = single_pass                     // alive while the originating envelope is in flight
lifetime = fixed duration("30s")           // alive for 30s regardless of traffic
lifetime = window duration("5m") tumbling  // window with composite keying (key, window_id)
```

`duration("2h")` constructs a value; it does not read the clock and has
no side effect. The runtime compares that declared duration with ticks
from `system_clock`. Production supplies real ticks, while the Lab
supplies virtual ticks. A bare duration is interpreted as an idle TTL.
The structural words (`persistent`, `single_pass`, `fixed`, `window`)
belong to the DSL, and their duration arguments are ordinary CEL
values. When the lifetime ends, the optional `on stop` handler fires
before the slot is reclaimed.

**2. Event-driven** — a handler executes `stop`. The slot
is reclaimed at the end of that transaction, the next envelope for the
same key creates a fresh instance.

```rastok
on checkout {
  out = { … }
  stop           // explicit end after this handler
}
```

Use `lifetime` as a fallback (e.g., 30min idle TTL for safety) and
`stop` as the normal close path for event-driven workflows
(carts, sagas, multi-turn conversations).

There is **no automatic cascade between partitions**. If a tenant
is deactivated, its sessions idle-out naturally. When precise
coordinated cleanup is required, write an Operator with an
`on stop` handler or use `stop` from a relevant event
handler. Cascade is a footgun — silent destruction of referenced
state is the source of many bugs. TTL + explicit `stop`
covers all practical cases.

### 3.5 Stdlib partitions

Frequently used partitions are pre-declared by the stdlib:

```rastok
partition tenant   { key = meta.app.tenant_id ;  lifetime = persistent }
partition session  { key = meta.app.session_id ; lifetime = duration("2h")         }
partition request  { key = meta.app.request_id ; lifetime = single_pass }
partition user     { key = meta.app.user_id ;    lifetime = duration("24h")        }
```

These are conventions, not magic. The DSL is open: declare any
partition inline with a domain-specific name (`device`, `workspace`,
`conversation`, …).

### 3.6 Concurrency by key

Two envelopes with different keys at the same partition are
processed **in parallel**. Two envelopes with the same key are
**serialized** in arrival order at any Operator bound to that
partition. This is the standard Flink/Kafka Streams concurrency
model — what makes per-key reasoning tractable and per-entity
scaling predictable.

---

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
└── Host                      (one machine or K8s pod)
    └── Runtime               (one App deployed)
        ├── Coordinator       (parent process, 1 per Host)
        └── Workers           (child processes, N per Host)
```

**Host** is the machine-level unit. The word "Node" is reserved for
graph elements (Operators, Connectors, and sub-Graphs — §2), never
for machines.

The **Coordinator** is the parent process on a Host (the Chrome /
postmaster pattern): it owns the inbound listeners, routes each
arriving envelope to the **Worker** that owns its key, and supervises
the Workers. Workers hold the in-memory state for their keys and talk
to external systems directly — inbound is centralized and key-aware,
outbound is decentralized, and once an envelope enters a Worker it
stays there for the full graph traversal. The process-level contracts
belong to the future runtime-architecture deliverable.

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

Horizontal scaling is achieved by **graph replication + sharding
by key** — the pattern Kafka Streams, Cassandra, and Orleans have
used at scale for a decade.

```
Cluster
└── App deployed on N Hosts
    ├── Host 1  (owns shard 0..K/N)
    ├── Host 2  (owns shard K/N..2K/N)
    └── Host N  (...)
```

Each worker instantiates the **whole graph** but processes only its
slice of keys. The slicing is a fixed function: `shard =
hash(partition key) mod N`, with N declared at deployment — a key's
shard never changes; only shard→worker assignments move.

A **shard** is the runtime-level slice of the keyspace assigned to
one worker (a Host runs one or more workers). A DSL `partition` is a
*named dimension*; its keys at runtime are distributed across the N
shards.

An envelope arriving at any worker is forwarded to the worker that
owns its key's shard — explicit, observable, journaled forwarding
with a durability receipt, not a hidden RPC. The Coordinator is a
pure control plane: it holds the placement map and never touches an
envelope.

**The system is CP** in the CAP sense, for keyed state. When a Host
fails, its shards are temporarily unavailable until reassigned, and
their state is rebuilt from the durable journal per the graph's
declared durability scope — no active replication of `var` state
across Hosts beyond the journal's own replication, the same
rebuild-from-persistence pattern Erlang/OTP follows. Split-brain is
prevented by **epoch fencing**: shard reassignment increments a
write epoch, and stale-epoch writes are rejected at the durable
boundary. Membership, lease, and rebalancing contracts are deferred
cluster-protocol work and are not implementation claims.

**No operator-level placement.** Rastok deliberately does **not**
support placing different Operators of the same graph on different
Hosts (the Flink-style operator placement model). Each Host runs
the whole graph for its keys. This trades expressive power for
operational simplicity — no cross-Host shuffles inside a graph, no
network channels between Operators, no per-operator parallelism
tuning.

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

Partition lifetimes combine a declared duration with clock events.
The duration is a pure value. Expiration occurs only when a Clock event
advances the relevant timeline.

### 5.1 Sequence

State is read-modify-write, imperatively, within a transaction. A field
`var x` is read by its bare name `x` (or `$x` inside `jq { … }`).
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
operator stats per session {
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
| Cleanup of an idle state slot | `lifetime = duration("2h")` on the partition |
| Tumbling window aggregation *(sliding windows are deferred)* | `lifetime = window duration("5m") tumbling` on the partition |
| Rate limit "max N per minute" | Partition with `lifetime = window duration("1m") tumbling` + a counter |
| Deadline / timeout for a saga | `lifetime = fixed duration("30s")` + `on stop` handler |

Explicit `Clock` is reserved for patterns where an
Operator actively emits or polls on tick — periodic metrics,
external state polling, heartbeats.

Production and Lab execution differ only in the source of clock
events. In production, `system_clock` emits events derived from real
time. In the Lab, `advance duration("2h")` asks the virtual clock to
emit the same relevant events without waiting two hours. Those events
travel through the same lifetime, window, and handler logic.

For example, an idle lifetime records the last activity event and its
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
| Connector | Mock — replaces the partner, never the bookkeeping |
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
checks whether real adapters and partners still honor those contracts.**
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
error asserted on `app.err`, an `expect nothing`. Its scenarios are
acceptance criteria a product owner can read: "the decision may arrive
first," "a rejection is a business outcome," and "no decision within
48 hours becomes the technical `JoinExpired` error." Nothing waits
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
  mid-lineage envelopes). `app` stays pre-bound (`app.err` = the root
  error sink), and node references in mocks and expectations are
  root-relative qualified paths (`sessions.codex`).
- **A scenario is a named slice of that space**: when executed, it becomes
  one deterministic trajectory through the graph, with concrete fixture
  steps, mocked Connector outcomes, and expectations.
- **Mock/Inject/Expect** reads vertically as a Gherkin scenario
  (simulation-semantics.md is the normative spec of this surface).
- **Mocks mirror Connector boundaries**: a mock is written in the same
  handler grammar as a node body, but it replaces only the external partner,
  not the Connector's journal bookkeeping. The graph in test is the same graph
  in production.
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
    profile = "bank-sandbox"          // exactly one; a constant CEL text —
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

### 9.7 Tagged blocks and guest languages

Many DSLs and host languages have explored the pattern of
*embedding* a guest language (jq, SQL, regex, shell) as a tagged
block, with the host parser treating the body as opaque. Tcl, Lua
inside Nginx, jq filters embedded in scripts, SQL inside JOOQ —
all variations on the same host/guest pattern.

**What Rastok takes:** the tagged-block syntax (`jq { ... }`,
`sql { ... }`) for embedding guest languages inside the
DSL without requiring the host parser to understand them. The guest
language has its own evaluation rules; the DSL turns the body into a
`GuestProgram` for the corresponding `GuestExecutor`.

**What Rastok leaves behind:** the open-ended escape-hatch posture.
Each tagged block in Rastok has a fixed frontend/executor profile (jq,
SQL, …) and typed boundaries; you cannot smuggle an unregistered
language implementation through a tag. A guest that performs external
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
order within the partition** (processing-time) and bounds them by
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
  partition keys.

- **Agent runtimes and LLM orchestration** (LangGraph, Semantic
  Kernel, ADK). The Operator/Connector model with per-session
  partitions, mockable Connectors, and `on (a, b)` join handlers for
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
4. **Every piece of state is attached to a partition. Every
   partition has a key and a lifetime. No parent, no cascade. The
   envelope is the current message, not a partition.**
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
14. **A partition instance has three lifecycle phases: starts
    (lazy, on first envelope for a key, transaction `on start`), runs
    (handlers fire on events), stops (event-driven via `stop`
    action, or time-driven via `lifetime` expiration transaction
    `on stop`).**
15. **Two keywords, scope by position: `val` for any immutable
    binding (App-level, Operator-level, or handler-local), `var`
    for partition state with read-modify-write per transaction. Position
    determines lifetime.**
16. **Ports are implicit from the handler (`on X` ⇒ input, `X = …` ⇒
    output); `input` / `output` only pin a type or document. `in` /
    `out` / `err` are the default ports, stdio-aligned; Connectors
    declare none. `err` carries a statically-typed error set (the `E`
    of `Result<out, err>`); panics are supervision, never envelopes.**
17. **The `data` payload is statically typed by default; `variant` is
    the only dynamic type, narrowed before use. The type tree is
    scalar(encoding+logical) / struct / map / array / enum / union,
    anchored on Arrow. `table` is the relational view of
    `array<struct>`, not a type. Numbers are lossless until narrowed.**
18. **A handler is a transaction guard. `on (a, b)` is a join keyed by the
    partition; correlating two ports' content is a θ-join and demands
    a bound (`last` for memory, optionally `within` for staleness).
    OR across ports is separate handlers, not `||`.**

19. **The Lab is the App's twin, never woven into the deployed graph:
    simulations close graphs, experiments exercise real boundaries,
    challenges search bounded spaces, and future theorems may establish
    proofs.**

These maxims are not arbitrary slogans. Each one resolves a specific
architectural ambiguity, and contributors are expected to know them.
