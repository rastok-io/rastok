# Rastok: White Paper

<!-- Public narrative: rastok-strategy/NARRATIF.md,
     "Version courte de référence, deux paragraphes", English block, 77c4637.
     Keep this copy verbatim; propose wording changes in the source. -->

> An integration rarely fails on the happy path. It fails on the edge
> cases: partners not ready, late or duplicate replies, implicit contracts,
> test scenarios that drift. With large language models and AI agents,
> writing a flow has never been faster; proving it will hold in real
> conditions has become the real bottleneck. **Rastok is a simulation-first
> integration platform.** An application's flows, their contracts and their
> failure scenarios are written together, in a dedicated, typed language
> versioned in Git. The test environment is source code: partner faults,
> delays and silences are written next to the program. The *same* program
> runs first in a simulated world, then, without a rewrite, in the real one.
> You build and prove an integration before the partner systems are
> available.
>
> Every run leaves a journal that can be replayed without repeating its
> external effects. An incident becomes a scenario that protects the next
> changes. The journal is also a record an auditor can read. Before anything
> runs, you know which systems will be called, under which conditions, and
> to what end. The compiler refuses a flow that leaves an outcome with
> nowhere to go. A scenario proves the flow holds against the partner as
> described; acceptance, with the same program, checks the description.
> What an assistant writes goes through the same compiler and the same
> scenarios as what a human writes; review focuses on the contracts and the
> scenarios.

**Status of this document**

> **Format-24 (2026-10-24), diagnostic recording 5.**
> The Node/Executor/Extension model, host-terminal Loop, exact-instance closure,
> native outbound Call Retry and typed measurement retention are executable.
> The [C4 qualification](docs/architecture/retry-c4-qualification.md) records
> the qualified native-Retry base. The current lifecycle trigger movement is in the
> [lifecycle inventory](docs/architecture/lifecycle-registry-identity.md).
> The preceding Process identity movement is in the
> [Connector inventory](docs/architecture/connector-rework-identity.md),
> with verification in the [qualification report](docs/architecture/connector-rework-qualification.md); external
> [opening gates](docs/architecture/opening-showcase.md) remain separate.
> Process uses `std/process@6`: job is an outbound Call, service a resident Session.
> Earlier [R9](docs/architecture/rework-r9-qualification.md),
> [host-terminal](docs/architecture/loop-host-terminal-v17.md) and
> [exact-instance closure](docs/architecture/instance-stop-command.md) ledgers
> preserve their historical version movements. This is not a production-readiness claim.
>
> **Reading examples.** Linked `.rkg` programs in the
> [example catalogue](docs/examples/README.md) are checked and simulated.
> Inline snippets are excerpts, not standalone compilation units. Sections
> explicitly marked *future design* do not describe commands available today;
> optional-access expressions, jq/SQL guests, Timer, Calendar,
> Saga, experiment and coverage generation remain named gates.

> **Rastok is the boxes-and-arrows paradigm, rebuilt as an engineering
> discipline.** The flows that organizations already draw (integrations,
> sessions, agents) are written in a dedicated, typed
> domain-specific language (DSL), versioned in Git, exercised through
> deterministic simulation, and replayed from a causal journal. The same
> source is designed to move through a standard GitOps delivery process,
> from one native runtime process to a sharded deployment.

Rastok is an **open-source runtime and typed graph DSL for stateful,
effectful integrations**. A graph connects existing services, models,
tools, data, and business processes through three Node declaration forms:

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

An **Envelope** is the immutable message carried by an edge. An **Operator**
transforms envelopes and may update private semantic state, but has no external
side effects. A **Connector** is the explicit boundary where the graph reads or
changes the external world. Both share the Executor mechanics of mailboxes,
transactions, journaled state and ordered emissions; their transitions come
respectively from DSL Dispatch and a versioned Contract plus Driver events. A
**Graph** is a passive scope that composes these nodes. In the Lab, controlled
models replace external effects while the same Connectors, Operators and
checked graph run. Their executable Contracts do not require physical Drivers
to be linked into a simulation build.

AI agents are a useful stress test: they combine long-lived state,
external effects, variable cost, and probabilistic results in one
operational flow. The same controls also apply to less probabilistic
integrations. When a particular model changes, the surrounding
flow still remains.

The economic aim is to reduce the work of operating a portfolio of
integrations, not just the work of writing one flow. That benefit remains
to be measured beyond the alpha's executable scope.

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
the implementation they describe. Conceptual inspirations and category-based
positioning live in the annex (§9), after the model they inform.

And for the reader who wants the model up front, in one formal
sentence:

> *Rastok is a strictly acyclic graph of typed Nodes whose logical instances
> communicate through immutable envelopes, whose Executors and temporal
> Extension managers commit through one causal journal, and whose external
> world is replaceable by executable simulations.*

Every word of that sentence is unpacked in §2–§6.

## About the name

Rastok is named after **Rastoke**, the watermill village of Slunj,
Croatia, built where the Slunjčica river branches into cascades before
joining the Korana. The Croatian name comes from *ras-* + *tok*,
literally "the flow, branching apart." For centuries the village has
channeled that branching water through its mills: every stream visible
from the footbridges, every drop doing work on its way through, the
whole thing legible at a glance and still running. A village-sized
dataflow graph, powered by water: the name is a homage, and a design
target.

---

## 1. Why Rastok

Enterprise integration projects fail for reasons that are human before they
are technical. The code may still be there when the knowledge needed to
operate it has gone:

- The person who understood the integration is unavailable, has left the
  project, or has finished the vendor assignment.
- Two teams understood the contract differently, and neither wrote it down.
- Unit or integration tests were never written, or no longer reflect the flow.
- An edge case that nobody anticipated reaches production.
- Connected components have different lifecycles, and a change eventually
  breaks their compatibility.

These causes will not disappear. Rastok keeps the flow, its versioned external
contracts, its fault scenarios and its journal in an artifact that can outlive
the people, the project and the vendor. The flow and the scenarios are source
code versioned in Git; the journal explains what a run did. Together they form
an executable specification that the next team can read and test.

### 1.1 Problems Rastok is designed to solve

Modern interactive systems, including AI agents, combine several
difficult operational properties:

| Problem | What commonly goes wrong | Rastok's contract |
|---|---|---|
| Long-lived keyed state | Session state is scattered across workers, caches, and correlation tables. | A Node selects a logical instance by key; an Executor owns the private state and mailbox, while a Graph scopes descendants and lifetime remains independent. |
| Events arriving out of order | Each flow grows its own correlation state machine and locking rules. | Bounded joins correlate typed envelopes by key. |
| External side effects | HTTP calls, processes, files, payments, and retries are hidden inside application code. | Every external interaction crosses a declared Connector. |
| Probabilistic components | An LLM may return different answers for the same request, with variable latency and cost. | The journal records each boundary outcome; the graph isolates and measures the probabilistic surface. |
| Time-dependent behavior | Code reads the host clock or sleeps, so tests wait in real time and cannot reproduce the same timeout race. | The Execution owns an explicit logical timeline: `now()` observes one transaction-fixed instant; internal deadlines execute today, while Timer/Calendar source Contracts remain future work. |
| Recovery and audit | Effects and state updates leave separate, incomplete records. | Transactions and boundary outcomes form one causal journal. |
| Operational policy | Every team reimplements retries, timeouts, supervision, backpressure, and cost metering. | The graph declares shared runtime policy once. |
| Change control | Visual canvases and custom glue are difficult to review, compare, and generate safely. | Typed text is versioned in Git, checked before execution, and projected into a diagram. |

Three questions summarize the intended result:

1. *Can you replay yesterday's incident along the same causal path?*
2. *Can you test the whole flow without calling the real LLM or bank?*
3. *Can you show which input caused each external effect?*

Time matters in all three. Reading the operating-system clock inside an
expression is an external observation, just like reading an API response. If
that value appears in neither the fixture nor the journal, a test must wait and
a replay may take a different branch. Rastok therefore makes the Execution timeline
explicit: `now()` reads one injected transaction instant, never the host
clock, and the journal seals that instant whenever it can affect behavior
(§5).

### 1.2 Why use a DSL instead of a general-purpose language?

Every example in this document can be written in Python, Java, Rust,
or Node.js. A developer, with or without an assistant, can write a
working version of the approval flow in §1.5 using a map,
two HTTP handlers, and a timer.

That is not the expensive part. **Writing one workflow is cheap.
Operating a portfolio of them for years is not.** Without a shared
runtime, each team must design, build, test, and maintain its own
keyed state, correlation, recovery, simulation, journal, supervision,
backpressure, metering, and diagnostics.

The benefit is organizational as well as technical: one set of
conventions instead of one per team; incidents that are replayed
instead of argued about; and less bespoke platform engineering around
every new flow.

> Rastok does not replace a general-purpose programming language.
> It provides the common platform that organizations otherwise build
> around their flows. The intended benefit is a shared way to build, test
> and operate a portfolio of integrations, not a measured scale claim.

#### 1.2.1 The test environment is source code

One item above deserves to be pulled out, because it changes daily
practice. In a flow platform or custom codebase, exercising a complete
integration against its real boundaries often means having a database,
the partner's API, a broker, credentials, test data, a staging
environment, and an available partner team. Testing edge cases (events out
of order, a 48-hour timeout, a crash right after a payment, a reply that
arrives too late) ranges from laborious to
practically impossible.

In Rastok, the world around the graph is part of the simulation
specification: supported partner interactions are scripted through their
Contracts, while logical time is supplied explicitly. The flow builds and runs on a laptop,
whole-world simulated, **before the real systems are available**, and
each edge case is expressed in a scenario (§1.5). This is not unit
mocking: the simulation
closes the *entire* world, controls time, arrival races and faults,
and produces a deterministic, replayable trace.

> In traditional integration tools, the test environment is
> infrastructure. In Rastok, it is source code.

#### 1.2.2 Why organizations already buy integration platforms

The demand for integration and workflow platforms is not that boxes and
arrows are impossible to program in a general-purpose language. It is that
*developing, deploying, supervising and maintaining a portfolio of bespoke
integrations* is expensive and risky. Rastok takes that
proposition, modernized: a textual source of truth instead of a
canvas, types, Git and code review, deterministic whole-world
simulation, a journal built for replay, LLM-friendly generation
(§1.4), with the visual graph still there, derived from the source
that actually runs.

The general-purpose language keeps its role: the *inside* of a
Connector (native code against the real world) is Rust; your
services stay whatever they are. Rastok occupies the layer between
them: the flow, its state, its time, and its evidence.

### 1.3 The Rastok answer

The core move is an inversion. In many visual-flow tools, the diagram
*is* the program. Even when its snapshots are versioned, a canvas
snapshot is neither an intentional, typed source a reviewer can read
nor a world that can be closed for deterministic testing. In Rastok,
**the program is text, and the diagram is a projection**, derived
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
2. **All contact with an external system crosses a declared Connector**
   (§2.7), including HTTP, processes and files today. Logical time is a separate,
   explicit Execution input; the future `Timer` and `Calendar` Connectors would
   introduce elapsed or civil time as graph events (§5). Mock the
   Connectors and control the timeline, and the graph becomes a closed,
   deterministic system. That makes simulation (§6) a first-class artifact
   instead of a best-effort test harness.
3. **A Node may select its own logical instance by key** (§3), independently
   of its lifetime. An Operator's `var` remains private to that Operator. Processing
   is serialized within one Executor instance. Parallel Workers and sharding
   across keyed boundaries remain future work (§4).
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

For example, begin with a Connector calling an LLM to classify support
requests. The journal records its answers and each original annotated usage or
monetary value, including those from a held Retry attempt. M1 supplies this
evidence, not a cost report or automatic price calculation. Universal commit
timestamps and per-call latency are not
yet recorded, so OTLP spans remain deferred. If refund
requests become a stable rule, move that case into a typed Operator with
statically declared outputs and keep the LLM only as a fallback for ambiguous
requests. Replay the Lab
scenarios and compare their traces before deploying the change. Each
iteration can reduce the work delegated to the model; any cost reduction
must be measured. Simulations check declared behavior; proving stronger
properties belongs to the future `theorem` level (§6.1).

### 1.4 The DSL in the LLM era: the constraint is the accelerator

A DSL has an adoption cost: one more syntax to learn, unfamiliar editor
support and fewer experienced developers. Coding assistants may change
that trade-off. Rastok tests the hypothesis that a constrained language
and executable scenarios make their output easier to check. Gains from
AI generation remain to be measured.

Ask an LLM for the approval flow in Node.js and it will happily
produce different plausible implementations: different state models,
different retry policies, different error conventions, different tests
and instrumentation, architectures that resist comparison. All of them
can work, but each demands line-by-line review and its own operational
conventions. Ask for a Rastok graph
and the generation is fenced by a grammar, a type system, a static
topology, one state model, a hard separation between pure logic and
effects, and one runtime. The result is homogeneous, reviewable
as text, simulable against the declared scenarios,
immediately journaled, immediately visible as a diagram. A malformed
graph is rejected by the compiler; the simulation detects deviations from
the scenario's expectations before anything touches the real world.

The intended result is an artifact shared by the simulator, journal,
debugger, auditors, the next developer and the next assistant. The economic
hypothesis concerns **operation**, not a promised generation speed:

> An assistant helps write the flow.
> Rastok aims to reduce the work of operating, auditing and evolving it.

This is also what makes the graph-generation workflow of §6.5
credible: the LLM proposes, and the simulations constrain and check
the proposal. A constrained language is not a cage; it is what turns
machine-written flows into reviewable ones: **what an assistant writes
passes through the same compiler and scenarios as what a human writes.**
The loop does not stop at generating a happy path. Each counterexample,
production incident, or failed experiment becomes another scenario
that constrains the next candidate. This accumulating executable corpus
is one of the project's central advantages.

### 1.5 A first taste: correlating an approval

Two independent events must be correlated in either order, with a business
deadline. A request arrives on one input; a decision arrives on the other.
Keying the deciding Operator by request id puts both in the same mailbox
context. Missing input becomes a typed `JoinExpired`, not a silent discard.

The full checked source and three executable scenarios are in
[expense-approval.rkg](docs/examples/expense-approval.rkg). This excerpt shows
the application logic; types, the outbound HttpClient and the Lab are in
that file:

```rastok
operator decide {
  input request: Request
  input decision: Decision
  output approval: Request
  output rejection: Rejection
  output expired: JoinExpiry
  instance = { key = data.request_id }

  on (request, decision) within duration("48h") match {
    decision.data.approved => approval = request.data
    _ => rejection = {
      request_id = request.data.request_id
      reason = decision.data.reason
    }
  }
}
```

The enclosing App exposes two typed inputs rather than inventing ingress
metadata or bypassing a real listener. Its wiring is explicit:

```rastok
request -> decide.request
decision -> decide.decision
decide.approval -> accounting
decide.rejection -> rejection
decide.expired -> expired
accounting.out -> nothing
accounting.exit -> nothing
```

The accounting example assumes an idempotent upsert service keyed by
`request_id`. Its mock proves graph behavior, not the external service's
idempotency or a payment guarantee. Compensation and irreversible-step
contracts belong to future Saga work; this example does not execute them.

The Lab supplies the service response and exercises three cases:

- decision before request: one accounting call, no rejection;
- rejection: one domain rejection, no accounting call;
- missing decision: `advance duration("48h")` produces one expiry without
  sleeping or calling a real service.

For example, the timeout scenario is ordinary source:

```rastok
scenario approval_timeout {
  app.request = { request_id = "r-7"; amount = 990 }
  expect accounting.in == nothing
  advance duration("48h")
  expect app.expired where data is JoinExpired
  expect accounting.in == nothing
}
```

Each injection or time advance opens a new observation window. Asserting
absence in both windows proves that neither admission nor expiry posts the
request. Assertions read the real App boundary, not an implicit sink that
mixes outputs from unrelated producers.

Key selection, bounded correlation, virtual time, explicit outcomes and
inspectable causal history compose without another application state machine.
The current engine retains this evidence in memory. Durable storage,
automatic incident-to-fixture conversion and authored `crash`/`restart`
scenarios are separate future capabilities.

The [audit ledger](docs/architecture/documentation-audit-2026-09-09.md)
records the productibility defect this example exposed. It is now corrected:
timed/count-bounded joins make `JoinExpired` productible in both the compiler
and the independent plan validator. Removing its destination is an
`E-UNWIRED-OUTPUT`, not a dead-output warning.

### 1.6 A second taste: a session gateway

The same separation supports an agent gateway. The checked
[agent-gateway.rkg](examples/agent-gateway.rkg) connects WebSocket conversations
to resident JSONL children. A Graph keyed by `meta.ext.session_id` owns one
Process per conversation, retained across messages. Alice and Bob can use the
same request id without sharing either the child or the reply address.

The core wiring is short:

```rastok
clients.out -> conversation
conversation.out -> { clients.in; reply }
conversation.exit -> process_exit
clients.exit -> session_exit
```

Inside `conversation`, an Operator remembers the incoming socket coordinate
and restores it on each child response. Process remains a generic JSONL
transport; the client speaks the app-server initialization, thread and turn
protocol. The Lab uses explicit fixture inputs, scripts replies and
notifications, closes Alice's child, then proves Bob can continue. It launches
no agent and does not pretend to simulate a WebSocket handshake. A separate
local integration test uses two real sockets and harmless shell children.
The [composition ledger](docs/architecture/agent-gateway-showcase.md) separates
those proofs from paid-model or complete protocol qualification.

Three distinctions matter:

- **Session is not instance.** A socket's Session reference correlates its
  messages; `instance.key` independently selects Node-owned state. A keyed
  Graph scopes its children without acquiring an application mailbox.
- **Session is not job.** A resident Process response is a fresh Session
  ingress root. Restoring a socket's return coordinate does not rewrite its
  `origin_id` into a previous request's cause. Application request/turn ids,
  not the number of Session writes, identify agent jobs.
- **An EGRESS record is not a price.** Annotated quantities and amounts
  remain in their original journaled envelopes with their causal provenance.
  Missing consumption is unknown, never inferred from the number of calls.
  M1 implements no report, automatic pricing or subset arithmetic; OTLP export
  and budget enforcement also remain future consumers.

The earlier [WebSocket-to-job example](docs/examples/codex-websocket-gateway.rkg)
remains a finite-job demonstration, and the
[resident dialogue](examples/agent-dialogue.rkg) a smaller typed-adapter primer.
The larger [Teams design](docs/examples/teams-codex-gateway.rkg) is a historical
pre-v16 sketch. Public authentication, arbitrary protocol redaction, measured
model usage and operational concurrency policy are not supplied by the local
bridge. The HTTP usage/credential variant remains open; it must safely handle
missing JSON usage before claiming cost attribution. None of these examples
proves a shipped thousand-session gateway.

### 1.7 When Rastok is more platform than you need

Honesty about scope buys trust, so let us draw the lower bound: a
small Python script does not need a dataflow runtime. A serverless function
gluing two APIs, a plain CRUD service in front of a database, a
one-off migration or a notebook need not justify a DSL, and a
general-purpose language plus an LLM serves them perfectly. Rastok
below its threshold is ceremony.

The threshold is crossed when the *surroundings* accumulate: several
workflows sharing state and teams; effects that must be compensated
and audited; LLM calls whose cost and behavior must be metered;
long-lived sessions with processes attached; incidents that must be
replayed rather than guessed at; compliance questions that name data
paths. Each of those alone can be a library. Together they become a platform.
That is the moment the prototype begins its second,
invisible life as infrastructure. Adopt Rastok at the point where you
would otherwise start building the platform of §1.2.

### 1.8 Where this comes from: recurring graph traditions

Rastok's ancestry is the history of drawing systems as boxes and
arrows. These waves overlap; this is not a sequence in which one
approach replaced the previous one:

```
2000s+  SOA and ESB           flows in integration platforms and buses
2000s+  distributed data     flows in pipelines
2010s+  microservices        flows spread across deployed services
2010s+  workflow platforms   flows return as explicit DAGs
2010s+  low-code             flows reach a wider authoring audience
2020s+  agentic systems      flows include probabilistic calls, tools,
                             and human approvals
```

Two constants run through the story. First, **the drawing keeps coming
back**: whenever behavior is dominated by how data moves, engineers
and business people alike reach for nodes and edges. The demand is
structural: the graph is an artifact business, ops, dev, auditors and
coding assistants can read together. Operational questions ("where is the
message stuck?", "what is upstream of this error?") are questions
about the drawing; regulators literally ask for it ("show me how this
data moves and who touched it"); a map also gives a newcomer a way to
locate the relevant behavior. Second, **the era that suppressed
the drawing paid for it**: microservices scattered the flow across
multiple repositories. Distributed tracing and telemetry reconstruct the
graph at runtime, after the fact, from fragments.

Rastok's choice is **text as the authored source, a graph as its projection**.
The visual debugger makes recorded messages, instances and causal decisions
inspectable without turning the canvas into an editor. Compile-time contracts
and deterministic Lab scenarios are part of the same workflow.

This is a design position, not a ranking of current competitor features.
Visual integration tools, code-first workflows and stream processors all have
overlapping capabilities. Rastok does not infer production maturity from an
attractive diagram, nor claim that competing products lack testing or Git
integration. Its own durable storage, cluster operation and wider Connector
catalogue remain future work. The annex (§9.10) compares intended use cases,
not a versioned feature scorecard.

### 1.9 Who this document is for

- **Backend developers and SREs** writing the actual graphs and
  connectors.
- **Product managers and product owners** writing simulations as
  specifications.
- **Formal-methods reviewers** assessing the model and the invariants
  it aims to make checkable: approvals, delivery guarantees, absence
  of deadlock (the future challenge and `theorem` levels, §6.1).
- **LLM-assisted developers** generating graphs from simulations.

Rastok deliberately synthesizes three traditions that have historically
lived apart: deterministic dataflow, supervised actors, and modern keyed stream
processing, applied to long-running
conversational sessions with LLMs, process-backed agents, multi-tenant
SaaS. The full genealogy (what was taken from each tradition and what was
left behind) and the practical positioning by category live in the annex (§9).

---

## 2. The Core Model

This section builds the model from the inside out: the message
(§2.1), the node that processes it (§2.2), the handler that fires the
processing (§2.3), the state it may keep (§2.4), the expression and guest
languages a handler computes with (§2.5), the topology that connects nodes
(§2.6), and the place where the real world's loops are absorbed (§2.7).

A Rastok App is a graph that compiles into a target-neutral
`ExecutionPlan`. The plan is an immutable description of a state
machine. The Rastok runtime executes that state machine in its
Execution.

The first runtime is native: it is written in Rust and compiled for the
underlying operating system and hardware. The Execution is the interpreter
of the `ExecutionPlan`; the runtime itself is not interpreted and requires
no separate managed runtime.

The plan boundary leaves room for future execution forms:

- a self-contained native bundle that packages a plan with the runtime;
- the same Execution compiled for a browser or edge environment;
- a Wasm artifact for constrained deployment targets;
- ahead-of-time lowering for a proven need.

These are architectural directions, not current product commitments.

Although cycle checking may expand composition into one spatial DAG, the
validated plan retains the parent-linked Graph/Operator/Connector owner tree.
That tree is semantic: it locates Graph boundaries, instance frames, private
state and lifecycle. A compiler may flatten passive routing, but it may not
erase an owner and later attempt to reconstruct containment from a path.

Rastok uses one closed structural hierarchy:

```
Node
├── Graph       — passive composite scope, wired by name
└── Executor    — bounded atomic transition owner
    ├── Operator   — transition authored as DSL Dispatch
    └── Connector  — owns a Contract and crosses an external boundary
```

An Operator reacts to envelope arrivals on its inputs, optionally reads
and updates its private `var` state, and emits zero or more envelopes. A
Connector owns the same mailbox, transaction, semantic-state and ordered-
emission mechanics, but its transition is defined by its versioned Contract
and typed Driver events. Boundary attributes such as URL, path, command,
codec, or schedule configure that Contract. A Graph contains Nodes and edges,
owns instance/lifecycle scope, and never acquires an application transaction.
An Extension may attach an orthogonal protocol such as App or Loop to any
admissible Node.

### 2.1 The Envelope, its data, and the type system

Every message flowing through the graph is an **envelope** = `meta + data`:

`meta` is the lightweight routing and provenance surface; `data` is the
single carried payload. The postal metaphor fits: `meta` is what's written
*on* the envelope, `data` is what's *inside*.

`meta` has exactly three zones. `meta.sys` holds the runtime's read-only reserved fields —
identity, lineage and graph version: four fields, reminted at each emission.
`meta.app` is the application's writable zone — its fields inferred
from the graph's own writes, with types unified across writers — and
`meta.ext` is the external-boundary context, written by connectors
(their contract sheets declare its fields) and rewritable by
handlers. There is no bare form: `meta.x` resolves to nothing.
`meta` leaves are typed scalars (or scalar arrays) — composites belong in
`data`. The normative field list, read-only rules, id derivation, storage
form and Extension-context boundary live in
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
but bytes are the logical type only for opaque binary data. Graph-born values
use ccbor; external raw bytes keep their reading port's bytes/text distinction.
Arrow buffers may remain opaque; Avro is a schema/import-export projection. Pure
routing therefore does not pay parse cost. The exact authority is
[docs/spec/types.md](docs/spec/types.md), with host syntax in
[docs/spec/grammar.md](docs/spec/grammar.md).

Standard Connector types, including `FileExit` and `ProcessExit`, are available
by name without imports or copied declarations. They use the same logical
definitions as the Connector ports; aliases do not create a second identity.
A module/import system remains a later design discussion, with no syntax or
delivery date decided. The current names are listed in
[stdlib.md §1.2](docs/spec/stdlib.md#12-standard-types-in-source).

### 2.2 The shared Node shape

Every declaration creates one **Node**, but Node has only two structural
branches:

```text
Node
├── Graph       passive scope: typed boundary, children, wiring, lexical vals
└── Executor    mailbox + semantic state + atomic journal transaction
    ├── Operator    transition described by DSL Dispatch
    └── Connector   owns one Contract; transition described by it and Driver facts
```

This is why Operator and Connector deliberately have mailbox, ports,
transactions, state deltas, ordered emissions and atomic commit in common.
They differ in the provenance of their transition. An Operator may declare
private `var`; a Connector's replayable state is the core-owned exchange and
Contract fold, while its socket, process handle or pool remains reconstructible
Driver state. A Graph owns address, instance, lifetime, Closing and containment,
but no application mailbox, handler, `var` or transaction. Its lifecycle facts
are controller records, not Graph application code.

The annotation after `:` has a closed meaning:

```rastok
graph agent : App + Loop { ... }
operator retry : Loop { ... }
connector model : HttpClient + Loop { ... }
```

A Connector begins with exactly one versioned **Contract**. `App`,
`Loop` and future `Saga` are **Extensions** attached to an owner. An Extension
may contribute attributes, bindings, manager commands, admission constraints
and—only when the protocol needs one—ports. Extension order is not semantic;
incompatible contributions refuse instead of acquiring precedence. The
Connector attribute `behavior = deterministic | probabilistic` keeps its name
and is unrelated to Extension.

The port keywords are `input` and `output`. `in`, `out`, `err` and `exit` are
conventional, non-reserved names resolved like every other port. Every used
port has one effective type. An output which may be produced must have an
explicit disposition: another input, a Graph boundary output, an Extension
manager command, or `nothing`. A statically and independently re-derived
impossible output needs no destination.

Only Operators contain handlers:

```rastok
operator classify : Loop {
  input in: Request
  output accepted: Request
  output err: union { LoopExpired { last_state: Request } }
  Loop.terminal = [accepted, err]
  var seen: int = 0

  instance = { key = data.tenant }
  lifetime = { kind = idle ; after = duration("2h") }
  Loop.fuel = 3

  on in match {
    data.valid => { seen = seen + 1 ; accepted = data }
    _          => Loop.again
  }
}
```

A Connector contains Contract attributes but no DSL handler or author-owned
`var`:

```rastok
connector model : HttpClient {
  url = "https://example.test/model"
  codec = json
  idempotent = false
  behavior = probabilistic
  timeout = duration("10s")
}
```

A Graph exposes a boundary and wiring. It may carry `instance`, `lifetime`,
`closing`, lexical `val` and Extensions, but executable behavior lives in its
children or Extension manager:

```rastok
graph conversation : Loop {
  input in: Request
  output out: Answer
  output err: union { LoopExpired { last_state: Request } }
  instance = { key = data.session_id }
  Loop.fuel = 4

  in -> model -> decide
  decide.retry -> Loop.next
  decide.answer -> out
  model.exit -> nothing
  // Child declarations and their other dispositions are omitted in this excerpt.
}
```

No public Rust `Executor` trait is implied. Executor is first a semantic
category; shared concrete mechanics are consolidated only where they enforce
this contract or serve a real second implementation.

### 2.3 Event-driven semantics — handlers as transaction guards

An Executor is an event-driven state machine—not an FP function. A transaction
is

```
transaction = (trigger, state_in) → (state_out, emissions)
```

atomic: consumption, semantic state delta, ordered emissions and durable
manager commands commit together or not at all. An Operator obtains that
transition from one `Dispatch` handler. A Connector obtains it from its closed
Contract and a typed Driver event; the Driver never edits state or the journal
directly. Graph has neither form.

**Triggers.**

| Trigger        | Fires when                                                          |
|----------------|---------------------------------------------------------------------|
| `on a`         | an envelope arrives on port `a`                                     |
| `on (a, b, …)` | envelopes are present on all named ports (a **join**), zipped by arrival order within the owning Operator instance |

One Operator has at most one handler per trigger. OR across different ports is
expressed by separate handlers; alternatives within one trigger belong to its
`Dispatch`, using a closed or ordered-open `match`.

**`where` has two deliberately different readings.** On one input it is a
filter sugar: a false predicate consumes the envelope in an otherwise empty
transaction, exactly like `match { P => body; _ => nothing }`. On a join it is
an admission condition: false consumes no candidate, advances no cursor and
commits nothing. `within` and `last` bound joins; `having` remains a named
future gate. This asymmetry is explicit because filtering one event and
matching a tuple solve different problems.

`match expr { Case => ... }` is closed, disjoint and exhaustive for tagged
unions, enums and bools. `match { predicate => ...; _ => ... }` tries pure
predicates in source order. In both forms exactly one arm executes. Match is
the dispatch of a block or arm, never control inserted after an emission or
state mutation.

**Join key = owner instance.** Every envelope reaches one logical instance of
the Operator before it can enter the join buffer, so `on (a, b)` correlates in
that address for free. There is no second `by` key authority. If a flow needs
a different correlation domain, the Operator declares the corresponding
`instance.key`. The §1.5 approval graph is exactly this: both legs select the
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

**Future guest-language blocks in multi-input handlers.** In a single-input handler a
jq block's `.` is the input envelope; in a join there is no implicit
`.` — reference envelopes by port name (`a.data.x`, a Rastok expression) and scope
a jq block to one input when needed (`jq!(a) { … }`). See §2.5 for the
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

**Lifecycle.** Lazy `l:start`, `l:close` and `l:stop` facts exist for every
owner, including Graph, but only an Operator may author `on start` or `on
stop`. A lifetime deadline or bare `stop` requests Closing after the current
transaction commits. Contained Executors close inner-first; an Operator
child's `on stop` may still route an emission through its siblings before the
parent Graph's final `l:stop`. A Graph therefore has lifecycle without
pretending to execute application handlers.

**No implicit `on tick`.** `now()` may observe the fixed civil time of an
already-running handler, but it does not create a firing. When elapsed time
must itself trigger work, the future Timer Contract supplies an event (§5).
Timer/Calendar sources are not executable yet; this is the intended shape:

```rastok
operator stats {
  instance = { key = meta.app.session_id }
  input  tick : timestamp                // a Timer event's payload is the timestamp
  var count: int = 0

  on event { count = count + 1 }         // scalar state update: Rastok expression

  on tick  {
    metric = { meta.app.count_per_min = count }
    count  = 0
  }
}

// wiring at graph level
connector minute_timer : Timer { every = duration("1m") }
minute_timer -> stats.tick
```

A join example (settle an order once both legs arrive, correlated by
the Operator's order-id instance):

```rastok
operator settle {
  instance = { key = meta.app.order_id }

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

The simulator controls the Execution timeline directly. A call to `now()` receives
that timeline's one fixed value for the whole firing, and a declared `Timer`
turns selected instants into ordinary INGRESS roots. Neither Operators nor
mocks read the host clock. A real Connector may observe time in an external
system — for example, a Postgres query may call `NOW()` — but its controlled
Lab Driver script must return a declared outcome instead of repeating that external
observation.

### 2.4 Executor state and Operator-local `var`

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
  instance = { key = meta.app.session_id }
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
position admits ambient environment, network or filesystem I/O. Firing
handlers alone admit `now()`, which reads their fixed civil time, derived from
the run origin and relative logical instant, rather than an ambient clock.

**Emitting.**
- Each executed output assignment emits an envelope. Repeated assignments to
  the same output preserve source order and receive successive per-port
  `emission_index` values; they do not overwrite earlier emissions. The whole
  transaction still commits all emissions or none ([execution-model.md §4](docs/spec/execution-model.md)).
- `out = { … }` with only reserved roots (`data`, `meta`)
  is the envelope form: it **inherits** the input (meta + data) and
  overrides only the listed fields — a *patch* ; a whole-`data`
  assignment makes the data fresh.
- `out = { … }` with no reserved root is a data record (§2.1): it
  inherits the input's `meta` and replaces `data` with the new struct.
- `out = .` preserves the input data and heritable metadata, but creates a
  fresh emission identity and causal parent like every other emission.
- `nothing`, or `out = nothing`, emits nothing at that statement; it does not
  cancel preceding emissions. There is no implicit terminal verdict for Loop.

There is no `previous(x)` operator and no within-transaction snapshot trap:
a transaction reads the previous transaction's committed state at its start, then
proceeds imperatively. The synchronous-dataflow `pre` of Lustre/SCADE
applies at the **inter-transaction** boundary — each transaction is one cycle —
not *inside* a transaction (see §9.1).

**One owner, no remote reads.** An authored `var` belongs exclusively to the
Operator that declares it and is visible only in that Operator's handlers. A
child Node cannot read an ancestor's mutable state, and sibling Nodes cannot
address one another's cells by name. Keying a Graph creates a logical
containment frame; it gives no application mailbox or transaction to the
Graph. Contained Executors remain the serialization points.

Shared mutable behavior is modeled as an explicit stateful Operator with typed
command/result ports. Operations that must be atomic over the same state live
in that Operator's handlers and are serialized by its instance mailbox:

```rastok
operator account {
  instance = { key = meta.app.account_id }
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
the Executor transaction. Across Nodes, composition is envelopes plus the causal
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
| `var` | Operator-level | yes (read-modify-write per transaction) | current logical Operator instance; visible only in that Operator's handlers |

```rastok
graph my_app : App {
  val DAILY_QUOTA   = 1000               // App-level constant
  val SLACK_CHANNEL = "#alerts"
}

operator pricing {
  instance = { key = meta.app.tenant_id }
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
because they are immutable lexical context. A Graph cannot declare `var` or a
handler; mutable state belongs to contained Operators, while a Connector's
Contract/exchange state is core-owned and journal-derived.

These scopes also define evaluation phase. In the target model, Unit/App boot
`val`s are resolved once for the deployment. The M1.8 A4 executable subset is
narrower: exact environment/secret references are open only for the declared
HttpClient URL/bearer settings, while general boot `val`s remain a named gate.
A Node-level `val` is derived once for each new
instance from enclosing immutable values and that owner's local
`instance.key`; it cannot read `data`, `meta`, or mutable state. Handler-local
`val`s are transaction bindings. `instance.key` is a reserved contextual root,
not a name that searches upward: a child consumes a parent Graph key only
through a lexical Graph `val`.

Connector contracts apply the same distinction by declaring the latest
evaluation site a field permits: plan, boot, instance, or invocation. Checking
selects the earliest site where the written expression closes. Profile,
port-type, idempotence, behavior, and other plan-shaping attributes permit only
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
  current transaction-local value, including preceding assignments. The
  guest-language spelling remains prospective until that guest executes.

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

A future admin surface would push envelopes to `settings.admin` through an
explicit inbound Connector; neither Rastok Cloud nor UI data editing is shipped.
The optional-access expression above is a future expression-subset example;
consumers wire `settings.changed` or request `settings.read`.
Ownership and ordering are visible in topology instead of hidden behind a
nominal remote read.

### 2.5 Computation: Rastok expressions, tagged frontends, helpers, and primitives

#### 2.5.1 The core expression language and its guests

The DSL itself is deliberately **declarative** — nodes, ports, handlers,
wiring. Intrinsic computation uses Rastok expressions. Structural or
specialized computation may use registered tagged frontends with fixed
compilers/executors and typed boundaries. jq is the initial specified document
guest; jq and SQL execution are both still deferred. The executable tagged
readers are JSON, text and base64, not general guest programs.
Additional guests are admitted only through the same contract.

**Rastok expressions — the intrinsic core.** Every expression position in the
DSL — a `where` guard, an instance-key expression, a future
conditional-emission selector, a state expression, an attribute value, a
simulation expectation — is one pure Rastok expression:

```rastok
on payment where data.amount > 100 && meta.app.region == "eu" { … }
count = count + 1
key   = meta.app.session_id
```

The expression language is *total*: it always terminates, has no side
effects, and cannot express loops. That is the point — every expression in a graph
can be type-checked, costed, and simulated without running anything.
Absence is explicit: a bare read requires the field to be present. Optional
access such as `data.?discount.orValue(0)` is a future expression-subset
surface, not an executable fallback in the current build.

**`jq! { … }` — future document reshaping.** When the job is structural — build
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
`GuestRunner`. Each frontend/executor profile is fixed, every block has
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
  `operator <name> { … }` form expresses a transition using Dispatch.
  Reducer, Aggregator and Joiner are descriptive patterns over `var`,
  `instance.key`, lifetime and bounded joins—not Node categories. Routing,
  dispatch and fan-out name real language operations rather than nominal
  pseudo-types. A future pinned jq or Wasm transition must fit this ontology
  without reviving a `NodeType` axis.
- **Connectors are runtime primitives.** They bridge the graph to
  the outside world through Driver code (HTTP libraries, process
  spawning, database drivers, file I/O). A new Connector requires an executable
  semantic Contract and pure exchange rules; port declarations in DSL alone
  are insufficient. Simulation binds a model of the external world. Real
  execution additionally requires a registered physical Driver. This is the
  *syscall surface* of Rastok.

In OS terms: Operators are userspace functions composed from DSL primitives;
Connectors are syscalls. The stdlib names, Connector contracts, `behavior`
rules and pure guest-program contract are specified in
[docs/spec/stdlib.md](docs/spec/stdlib.md).

**Effects on Connectors only.** Operators are pure by construction — they
compute, route, mutate state inside the graph, but they do not write to the
outside world. The exact effect classes, compensation rules and Saga behavior
belong to [docs/spec/effects-and-sagas.md](docs/spec/effects-and-sagas.md).

**Extensions** attach orthogonal protocol to one Node. `App` marks the public
root Graph; `Loop` owns finite temporal recurrence; `Saga` is the next named
direction. Their generic admission and composition rules live in
[docs/spec/extensions.md](docs/spec/extensions.md), with the specific contracts
in [effects-and-sagas.md](docs/spec/effects-and-sagas.md) and
[loop.md](docs/spec/loop.md).

### 2.6 Topology: static DAG with bounded case routing

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
- **Workflow analysis** — a finite spatial topology is a useful proof
  input, not a proof of deadlock-freedom or a finite state space. Broader
  soundness claims require explicitly bounded data, instances and effects.

Dynamism is recovered through finite forms whose possible destinations remain
static:

- Operator `Dispatch` selects exactly one checked match arm;
- an edge `match` selects destinations by the closed tag of the unchanged
  source value;
- `instance.key` selects a logical instance of a statically declared Node;
- a destination set `{ a; b }` creates one envelope and N delivery references;
- multi-emission `(a, b) = value` creates two ordered envelopes in one atomic
  transaction. The value is evaluated once, but the envelope identities are
  distinct because the output ports are distinct.

These last two forms are intentionally not aliases. A new subscriber to one
source emission must not change that envelope's identity, while two authored
emissions have an explicit deterministic order.

A keyed Graph remains an ordinary declared scope:

```rastok
graph conversation {
  input in: Request
  output out: Answer
  instance = { key = meta.app.session_id }
  lifetime = { kind = idle ; after = duration("2h") }
  in -> classify -> answer -> out
}

requests -> conversation -> responses
```

The topology contains one `conversation` declaration. At runtime, each
session key receives its own address and contained Executor state; the Graph
itself does not acquire a mailbox or transaction.

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
  variable, Operator-local state does across envelopes.
- **In external event/session sequence** (§2.7). A user replying turn after
  turn or a long-lived Session produces distinct typed facts rather than a
  spatial cycle. A finite outbound Call may use `+ Retry` to repeat one
  prepared operation under a checked terminal-case policy. This is distinct
  from an authored `+ Loop` iteration, not another spelling for it.
- **In the `Loop` Extension.** Finite temporal recursion—apply this Operator,
  Graph, or outbound finite Call again until it reaches a terminal verdict—uses
  the host's existing entry and outputs:

  ```rastok
  operator refine : Loop {
    input in: struct { remaining: int }
    output out: text
    Loop.measure = Loop.state.data.remaining

    on in match {
      data.remaining <= 0 => out = "done"
      _ => Loop.next = { remaining = data.remaining - 1 }
    }
  }
  ```

  Every application selects exactly one verdict. `Loop.next = value` constructs
  the next complete state envelope; `Loop.again` retains `Loop.state`;
  `Loop.done` closes without business data; a configured natural terminal
  output supplies Done; and `stop` closes the owner instance. Next, Again and
  Done are durable **manager commands**, not ports or `PhysicalEdge`s. Their
  production commits atomically with the Executor transaction; a separate
  manager transaction consumes the command only after run-scoped quiescence.
  Any delay is logical time after command consumption. The later `Admit` fact
  re-admits at the exact input which opened the run, so recursion is temporal
  and the spatial topology stays acyclic.

  A run derives from the exact opening admission, not merely the business
  `origin_id`; two deliveries fanned from one root can therefore open distinct
  runs in the same keyed instance. If the final permitted application asks for
  Next or Again, the manager emits `LoopExpired { last_state }` on the host's
  existing failure terminal instead of re-admitting: `err` for an Operator or
  Graph, `exit` for an outbound Call, or a resolved `Loop.expired = failed`
  override. Loop contributes no port. The terminal follows the universal
  disposition rule: its parent routes it or writes `-> nothing`.

  `Loop.fuel` is the structural termination witness and remains an explicit
  cap: fuel N permits at most N applications, first included. The proof tier
  also accepts the direct signed-integer measure above when every recurrent
  Dispatch leaf proves positivity and an exact literal decrease; the
  deterministic `no_std` validator re-derives the certificate. Opaque Driver
  progress requires fuel, and every Call still needs a finite completion
  contract. Loop proves bounded state progression, not safe repetition of
  external effects. An authored Again path that may repeat a visible Call
  without the plan-owned `idempotent = true` guarantee produces
  `W-LOOP-REPEAT`; it is not Retry's hard refusal. Optional `Loop.delay` changes neither fuel,
  Call timeout nor instance lifetime ([loop.md](docs/spec/loop.md)).

**Loop advances state; Retry repeats an operation.** `+ Loop` remains admitted
on finite outbound Calls under the Loop host rules. Use `+ Retry` when the
request must remain identical and a static list of terminal cases decides
whether another Call is allowed:

```rastok
connector partner : HttpClient + Retry {
  url = "https://partner.example/orders"
  behavior = probabilistic
  codec = bytes
  idempotent = true
  timeout = duration("1s")
  Retry.attempts = 3
  Retry.on = [TimeoutExpired, ServerError]
  Retry.delay = { kind = fixed ; after = duration("100ms") }
}
```

The author must justify that idempotence guarantee against the real operation;
a URL or partner key alone does not prove it. BeforeEffect terminals can be
retried without the guarantee; OutcomeUnknown and AfterEffect terminals require
it, otherwise `E-RETRY-EFFECT`. The request, resolved settings and idempotency
material are retained, not recomputed. Each attempt is journaled, but only the
final batch is delivered to the graph; the last failure remains its original
typed case, with no `RetryExpired` wrapper. Retry's delay starts at the recorded
logical Call closure. A Loop may contain a Retry Connector; same-owner
`Loop + Retry` remains refused. See [Extensions](docs/spec/extensions.md).

The maxim form: *cycles do not appear in the graph — they appear in
state, in events, and in sequence*. Every admitted `Loop` has a finite
termination witness, and its body has a separately checked completion bound.

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
catalogue to a stable versioned contract id such as `std/process@6`. The
Contract declares inbound/outbound roles, typed ports, attributes and
lifecycle rules. A physical Driver supplies real I/O; a Lab model supplies
scripted arrivals and outcomes through the same pure protocol and admission
seams. Simulation needs the latter, not a linked physical implementation.
Before execution, the runtime selects one implementation for each active
Connector Node. The deployment may name it explicitly; otherwise exactly one
registered implementation must match the resolved Contract, mode, direction and
exchange. Absence, incompatibility and ambiguity refuse before any Driver is
created. Registration order never chooses an implementation. Neither Driver
names, crate names, shared-library paths, build features, Hosts nor Workers
enter the plan or causal identity. Calls and Sessions remain owned by the exact
selected Driver object, not merely by a shared Contract id.

For example, the HttpServer ack model accepts decoded fixture requests at
`listener.out`, without opening a socket or adding an App input. Only committed
admission permits its scripted `202` acknowledgement. This tests the graph
under those assumptions; wire decoding, authentication and successful delivery
of a real response remain separate Driver conformance obligations.

```text
RKG Connector ──▶ checked Contract definitions ──▶ validated plan
                                                 (id + mode + direction + exchange)
                                                          │
static factories ──▶ available implementations ────────────┤
deployment ────────▶ selection by Connector Node path ─────┤
                                                          ▼
                                      RuntimeDrivers: exact object ownership
                                                          │
                                                          ▼
                                        Worker: Execution + journal + Drivers
```

Static composition is the implementation available today: embedded, minimal
and full standalone builds may link different Driver sets while consuming the
same plan model. A future plugin sidecar can publish a data-only capability
manifest — Contract ids, implementation names, modes and exchange capabilities —
before code is loaded, then
register an ordinary factory through the same boundary. Rastok does not yet
freeze a native plugin ABI, artifact format, trust policy or hot-reload
lifecycle. Discovery is physical assembly, not a second execution model.
The reference CLI makes that assembly visible: Process is linked by default,
an explicit `--file-root` adds File and supplies deployment data that never
enters the graph or plan identity, and hosted feature presets add the selected
HTTP/WebSocket Connector implementations without changing their Contract ids.
The full hosted image currently supplies one compatible physical implementation
per executable standard mode. The registry also accepts multiple named
implementations of one Contract, with explicit deployment selection when needed.
A reduced image refuses a missing required Driver before Execution construction.
The read-only `rastok connectors [--json]` catalogue projects the shared standard
definitions, the linked Driver manifest and the Lab manifest; it does not run
the conformance fixtures or perform I/O. The
[contribution guide](docs/architecture/connector-drivers.md) follows File from
its Contract through registration and conformance.

Hosted network Drivers use the same separation. The synchronous Driver/Execution
edge feeds one bounded private pump: finite work and result queues, a separate
priority control lane, and a shared byte budget. HTTP/1.1 and HTTP/2 currently
target Hyper with Rustls; HTTP/3 targets quiche with BoringSSL; WebSocket
messages target Tungstenite. Those are replaceable physical bindings, not
semantic Node types. They are enabled by static build features and all async,
socket, TLS and library values remain below the Connector module. The two TLS
stacks deliberately consume one exact compiled root corpus and the same
in-memory identity encodings. A minimal or embedded build omits them without
changing a plan. HTTP/1.1 Upgrade, HTTP/2 Extended CONNECT and HTTP/3 Extended
CONNECT are proved separately, then project onto the same generic Session
fold; a framing test for one transport is never evidence for another.

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
uses the same contract-specific request protocol and resolved result types, and replaces
only the post-journal external interaction. It never replaces the Execution or its
bookkeeping. Each Connector's Contract owns that shared pure request protocol;
the Lab composes contract-provided binders instead of dispatching on contract
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

The future Timer and Calendar sources have Event-shaped contracts. Every call has a causal
call identity. For a world-originated Call, the committed request root itself
is that identity and opens the obligation on the exact source Connector
instance; its sole terminal fact is an ordinary `l:reply` transaction, not a
second outbound call. Every session has a causal
logical session reference independent of its socket, negotiated transport and
physical placement. A Process job or database operation is a call; a
WebSocket connection and the resident Process service mode are sessions.
Neither automatically invents a Node-instance key.

For WebSocket, one server Driver owns many logical SessionIds and one client
Node instance owns at most one live Session. Each complete inbound message is
a bounded two-phase proposal; each graph-to-peer message is a durable write
before bytes. Fragmentation and control frames remain physical protocol state.
A full pump leaves the ordinary Worker mailbox claimed rather than creating a
second queue. The sole terminal is `l:exit`; no message or duplicate terminal
may follow it.

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
to termination wires `process.exit -> …`. The resident Process service mode uses
one Session per child incarnation: new input may open a fresh child after
death, but automatic restart and request replay are not implied. `meta.sys`
stays closed while call, resource, and Node-instance identities remain causal
journal facts.

---

## 3. Instances, Keys, and Lifetime

The old word “partition” bundled three different questions:

1. **Where does this envelope belong?** — logical instance addressing.
2. **When does that instance end?** — lifetime.
3. **Who can read mutable state?** — ownership and lexical scope.

Rastok keeps those concerns separate. A Node owns its address selection and a
separate lifetime may close it. Authored mutable cells belong only to the
Operator that declares them; Connector semantic state belongs to its Contract
fold, and a Graph owns neither. The normative detail is
[docs/spec/instances.md](docs/spec/instances.md).

### 3.1 One Node, optional key, optional lifetime

Every Graph, Operator, and Connector may carry the same common attributes:

```rastok
operator cart {
  instance = {
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
`instance` is an explicit singleton in the enclosing Graph instance;
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

`instance.key` is one Rastok expression evaluated exactly once when an
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
instance = {
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
owner that declares `instance.key`; it never means an ancestor key or null.

### 3.3 Logical address, state ownership, and joins

A logical address is:

```text
static Node path
+ dynamic key/time frames of keyed or temporal ancestors
+ this owner's dynamic frame, if any
```

Frames are ordered outermost to innermost. Every owner instance has derived
immutable bindings and lifecycle at that address. An Executor instance adds a
bounded mailbox, semantic state, open exchanges, scheduled events and cursors; a
Graph instance adds containment but no application mailbox or transaction. A
temporal Extension manager adds run-scoped durable commands without becoming a
Node. After closure, a later delivery may create a clean instance at the
same address. The runtime calls successive lives **generations**; generation
is the zero-based journal-derived ordinal of `l:start`, fences stale events and
deliveries, and never enters the logical address or call identity. Runtime and
journal facts refer to an instance with `(address, generation)`.

A `var` belongs only to its declaring Operator and is visible only in that
Operator's handlers. Keying a Graph does not create a transaction around the Graph and
does not expose mutable cells to children. Graph-level `val` bindings remain
lexical and immutable, so descendants may safely consume context derived from
the Graph's `instance.key`.

When mutable behavior must be shared, it is an explicit stateful Operator with
typed command/result ports. That topology makes ownership, serialization, and
failure visible; there is no nominal cross-Node state lookup.

A join correlates inside the logical instance of its owning Operator.
`where` refines candidates and `within` / `last` bound retention. There is
no join `by` override: a different correlation domain is expressed by the
Operator's own `instance.key`, keeping routing and correlation under one
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

- **persistent** creates no deadline;
- **idle** closes after mailbox-admission silence;
- **fixed** closes at a fixed age from the creating admission;
- **single_pass** admits one root and closes after its bounded work settles;
- **tumbling** selects the epoch-aligned processing-time box open at admission,
  independently of when the run started.

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
The first close request is a durable `l:close` fact. Arrivals from outside
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

Obligation scope matters: a pending Loop `l:loop` wakeup is generation-bound
and becomes stale when its instance closes; Saga compensation debt is a fold
over the journal and survives instance closure, restart, and shard movement.

### 3.5 Concurrency and future sharding

One logical Executor instance reference owns one bounded mailbox; at most one worker
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

**Future deployment design.** Only the single-process, one-worker, in-memory
engine is shipped. Spatial recovery from journal records is implemented, but
no durable storage backend, multi-Worker scheduler or clustered runtime is
claimed by this section. The durability syntax below remains deferred.

Rastok is designed for three execution scales, with the same `.rkg`
source remaining portable across them:

| Mode | Use case | Process model |
|---|---|---|
| `embedded` | edge AI, IoT, mobile, CLI tools | Single process, RAM only |
| `standalone` | single process/container, small SaaS | One Host with Coordinator + Workers, local persistence available |
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
assigns its sequence, constructs the complete inbound envelope, evaluates the first
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

Rastok keeps five related notions cleanly separated. Not
to be confused with the synchronous languages (Lustre, Esterel,
SCADE) from which Rastok inherits ideas: those unify these notions
under a single global atomic clock, while Rastok exposes them as
orthogonal mechanisms.

| Concept | Definition | Where it appears |
|---|---|---|
| **Sequence** | FIFO order of envelopes on one input | Stateful reads (`x` carries history) |
| **Logical instant** | The Execution timeline; one fixed value per firing | `now()`, timeout/lifetime/deadline arithmetic |
| **Timer event** | Elapsed time itself triggers one root envelope | Future `Timer` Connector |
| **Civil schedule** | Calendar date/time in an explicit zone | Future `Calendar` Connector |
| **Causality** | Happens-before partial order on the global trace | Journal and theorem reasoning |

Instance lifetimes combine a declared duration with journaled supervision
events.
The duration is a pure value. Expiration occurs only when the caller advances
the explicit Execution timeline to the relevant deadline.

### 5.1 Sequence

State is read-modify-write, imperatively, within a transaction. A field
`var x` is read by its bare name `x` (or `$x` inside `jq! { … }`).
At the start of a transaction `x` holds the value the previous transaction
left; assignments mutate it in place, so later reads see the new
value. The transaction commits atomically at its end. Each transaction is one
cycle — the across-transaction "value the previous transaction left" is the
Lustre/SCADE `pre` (§9.1), but *within* a transaction the sequence is
imperative. See §2.4.

### 5.2 Logical time, `now()`, and timer events

Start from the failure mode. If an expression calls the operating system's
clock directly, that value appears in no causal record or test input: two runs
can diverge, replay cannot reproduce the branch, and a test can only wait.
One ambient clock read is enough to break reproducibility.

Rastok closes that hole without pretending that time is absent. The host gives
the Execution an explicit logical timeline starting at zero, separate from
its immutable civil origin. At the start of a firing the Execution fixes one
instant; `now()` returns `origin + instant` as a wide `timestamp` throughout guard selection and body
execution. The expression evaluator only reads a reserved binding. It never
calls an OS API. If a checked handler can observe `now()`, the TRANSACTION
record seals that value, so replay consumes the same input rather than asking
either clock again.

Hosted execution observes civil time once at boot and advances from a
monotonic timer, so wall-clock corrections cannot move a running Execution backward.
The Lab chooses an origin and advances instantly. A microcontroller without an
RTC can still advance from a hardware timer with the epoch origin (or an origin
provided by its host); it can enforce durations without claiming knowledge of
the civil date. `Calendar` is the separate source for civil schedules.

`now()` observes a firing; it does not create one. A Node that must react
*because time passed* wires a `Timer` and receives ordinary timestamp
envelopes — the `stats` Operator of §2.3 is the canonical shape:

```rastok
operator stats {
  instance = { key = meta.app.session_id }
  input  tick : timestamp        // a timer event is just data on a port
  …                              // on tick { … } — an ordinary handler
}

// wiring lives at graph level — the port stays decoupled from its source
connector minute_timer : Timer { every = duration("1m") }
minute_timer -> stats.tick
```

Use `now()` for “what is this transaction's civil time?” Use `Timer` for
“run this handler after/every duration.” These are deliberately different
questions.

**Wiring belongs in a Graph body, never an Executor body.** Operators declare
handlers and Connectors declare Contract settings; their containing Graph
declares how those Nodes compose. A shared source is reused by
adding edges, not by touching the nodes:

```rastok
minute_timer -> a.tick
minute_timer -> b.tick
```

(There is no port-side `<-` shortcut. The explicit form keeps node
bodies pure and lets simulation overlays rebind sources without
touching node interfaces.)

For most patterns "do X when time passes", you do **not** need an
explicit `Timer` wiring:

| Pattern | Mechanism (no clock wire needed) |
|---|---|
| Cleanup of an idle Node instance | `lifetime = { kind = idle; after = duration("2h") }` |
| Tumbling window aggregation *(sliding windows are deferred)* | `lifetime = { kind = tumbling; every = duration("5m") }` |
| Rate limit "max N per minute" | keyed or singleton tumbling instance + a private counter |
| Deadline / timeout for a saga | the distinct future `Saga.timeout` Extension attribute |

Explicit `Timer` is reserved for patterns where an
Operator actively emits or polls on tick — periodic metrics,
external state polling, heartbeats.

Production and Lab execution differ only in who advances the logical timeline.
In production the hosted source maps monotonic elapsed time. In the Lab,
`advance duration("2h")` reaches the same deadlines without waiting two hours.
Internal deadlines run directly on the dated-event queue; a declared `Timer`
turns its own scheduled instants into inbound envelopes. Both paths use the
same ordering and causal rules.

For example, an idle lifetime records the last mailbox-admission event and its
declared duration. Advancing the Lab clock past that deadline emits the
same expiration event that production would produce later. The resulting
`on stop` transaction is therefore deterministic for the same plan,
journal, and logical-time inputs. Rastok does not claim that civil time itself
is deterministic; it makes every semantic observation explicit and
replayable.

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
(including future Timer sources) accordingly, supplies the transaction context observed by
`now()`, and asserts on the
observed output trace.

This is what allows Rastok to be asynchronous in production yet
synchronous-like under simulation: the simulator closes the graph
under controlled inputs and replays deterministically.

---

## 6. The Lab: Simulation, Experiment, and Verification

**Current versus future.** `simulate` and the read-only visual simulation
debugger execute today. `experiment`, `challenge`, `prove`, coverage generation,
authored crash/restart actions and automatic incident-to-fixture extraction do
not. Their descriptions below are design directions, not available CLI verbs.
The core recovery tests exercise recorded journals independently of Lab syntax.

Every App may declare its twin: the **lab** — at
most one *logical* lab per App, possibly split across same-name
fragments in several files. Its name is independent of the App name. It is the App's
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
| Graph | Simulation: targets the unit's App by default, or explicitly with `: App` |
| Connector | Mock Driver — replaces the external interaction, never the bookkeeping |
| deployed App + real profile | Experiment — meets the real world |
| a concrete execution | Scenario |
| an observation | Expect |
| Graph + Simulation | ⊨ Theorem *(reserved for future use)* |
| Operator | **itself** — pure logic is never mocked: it runs |

The key is **same program, different world**. Pure Operators run
unchanged. Connectors mark typed effect boundaries. An App execution
resolves effect intents against real systems; a simulation in the Lab
resolves the same intents through controlled Driver scripts, virtual time,
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
*declaration*, `theorem <name> { … }`, of a
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
outcome asserted on `decide.expired`, and an absence assertion with `== nothing`. Its scenarios are
acceptance criteria a product owner can read: "the decision may arrive
first," "a rejection is a business outcome," and "no decision within
48 hours becomes the typed `JoinExpired` outcome." Nothing waits
forty-eight real hours: `advance` moves the *logical*
timeline, and every time-dependent decision in the graph (join expiry,
Connector timeouts, lifetime and window closes) fires deterministically —
§5's explicit-time rule paying off.

Key properties:

- **A simulation is the graph's mirror, not a separate test language**:
  the graph says how the system reacts to envelopes; the simulation says
  which world surrounds it — inbound inputs, mocked Connector behavior,
  virtual time, failures, and expected observations.
- **A Lab contains mocks and scenarios directly.** An optional
  `simulation name { ... }` groups another mock world. Both forms target
  the unit's App; `: App` makes an explicit simulation's target visible.
  Sub-graph targets remain deferred. Mock and expectation paths are
  root-relative, such as `sessions.codex`.
- **A scenario is a named slice of that space**: when executed, it becomes
  one deterministic trajectory through the graph, with concrete fixture
  steps, mocked Connector outcomes, and expectations.
- **Mock/Inject/Expect** reads vertically as a Gherkin scenario
  (simulation-semantics.md is the normative spec of this surface).
- **Mocks mirror Connector boundaries**: a mock uses handler-shaped Lab
  Driver-script syntax, not a Connector-owned application handler. Its factory
  binds through the same exact contract id, roles and contract-specific request protocol as the real Driver. It
  replaces only the external interaction, not the Connector's journal
  bookkeeping. The graph in test is the same graph in production.
- **Intermediate assertions** (`expect` on any internal port) test
  transformations without isolating nodes; observation is read-only and
  legal anywhere.
- **Names carry intent.** Scenarios use descriptive identifiers, without
  string labels. Comments explain context in ordinary source text.

The executable forms are `expect port` (exactly one delivery),
`expect N port` (exactly N), `expect port == nothing` (absence),
`expect [N] port == value` (typed equality), and
`expect [N] port where predicate` (matching count). A payload compares in
full; an envelope pattern compares its written fields. The brackets denote
an optional count, not literal syntax. Successive expectations inspect the
same window until the next injection or time advance. Mock lookup replaces
a whole mock at the nearest scope: scenario, simulation, Lab.

`origin` declares the virtual timeline's civil anchor. Delayed mock
`after`, scenario `crash <node>` and `restart <node>` remain deferred;
core recovery tests do not imply an authored Lab crash command. See
[simulation-semantics.md](docs/spec/simulation-semantics.md).

### 6.3 Experiment — where the Lab meets reality

**Future design; execution is currently refused.**

An `experiment` runs the **real App under an external deployment
profile, with at least one real Connector boundary, in real time**. The
inbound side stays fixture-driven for now — injections still enter at
the source Connector's output — until a partner-side driver/lowering
contract exists. It lives in the Lab beside the simulations and reuses
their scenario grammar:

```rastok
lab checkout {
  simulation payments {
    mock bank {                       // in SIMULATION the bank is scripted
      on in { out = { status = "charged" } }
    }
    scenario accepted {
      orders.out = { meta.ext.order_id = "o-1"
                              data = { amount = 120 } }
      expect ledger.in where data.status == "charged"
    }
  }

  experiment payment_sandbox {
    // Deployment settings stay outside the Lab source.
    // `bank` is deliberately NOT mocked here : it is the real boundary
    // under test, reached through the profile's sandbox credentials.
    mock notifier {                   // boundaries NOT under test stay mocked
      on in { out = { delivered = true } }
    }
    scenario accepted {
      orders.out = { meta.ext.order_id = "o-2"
                              data = { amount = 120 } }
      expect ledger.in where data.status == "charged"
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
finitely enumerable. Future coverage tooling could produce the following
illustrative report; `rastok coverage` is not a current CLI command:

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
**The LLM generates candidates; the verifier decides.** This is an engineering workflow,
not a completeness proof: the DSL search space is constrained by a typed grammar — the LLM is
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
state machines), but not static topology: Rastok also fixes the physical graph. Its bounded
Dispatch selects among statically declared routes; it does not add edges.

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
edge is visible in the source, dynamism is bounded (conditional-emission and
keyed-Graph patterns plus inline broadcast `(a, b, …) = x`, §2.6), and all I/O crosses
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
`GuestProgram` for the corresponding `GuestRunner`.

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

### 9.10 Practical positioning: categories of use

This is a map of overlapping use cases, not a current product-feature audit.
No operational, performance, certification or ecosystem parity is claimed.

- **Integration and visual flows.** This category connects applications and
  partner systems. Rastok's authoring contract is typed RKG text, explicit
  outcomes and a read-only diagram over the actual plan and journal. Flows,
  contracts and Lab scenarios are versioned together.
- **Workflow orchestration.** This category coordinates external operations
  and their timing. Rastok currently provides finite Loop recurrence, bounded
  Retry and deterministic simulations. Civil scheduling, durable deployment,
  backfill semantics and Saga remain future work.
- **Agent orchestration.** This category composes models, tools and
  conversations. Rastok focuses on typed integration boundaries,
  keyed state, effect visibility and reproducible scenarios. A complete AI
  gateway still needs provider adapters and deployment/security policy.
- **Stream processing.** Keyed state and joins are relevant to continuous
  data flows. Rastok uses bounded processing-time correlation; it does not
  claim event-time/watermark semantics or large-scale analytics performance.
- **Safety-critical tooling.** Explicit contracts and analyzability matter
  when assessing a program. Rastok's narrow Loop proof is not a certification,
  hard-real-time guarantee or general theorem prover.

A useful evaluation compares a concrete flow, its declared fault scenarios
and its operational constraints. The runnable examples and qualification
ledgers are evidence for Rastok; future design sections are not.

---

## 10. Status

Rastok is experimental, not production-ready. The current format is
**package 24 / diagnostic recording 5**, with one Node table and Graph-owned wiring.
The approved witness movement and qualification are recorded in the [time-carrier inventory](docs/architecture/time-carriers-identity.md).
Process job/service modes share the Contract `std/process@6`;
File and the four HTTP/WebSocket Contracts use `@3`. These version registers
are independent. The [Connector identity inventory](docs/architecture/connector-rework-identity.md)
records the Process-dependent semantic hashes and Retry traces that change.
The earlier [Node realignment inventory](docs/architecture/node-realignment-identity.md)
records a package-only movement with unchanged causal identities. No public
release or compatibility reader is implied.

| Executable today | Qualification / exact limit |
|---|---|
| Node / Graph / Executor / Connector / Extension; recursive Dispatch; explicit destinations; case routing; ordered multi-emission | [R9 ledger](docs/architecture/rework-r9-qualification.md), Registry-free validation and hostile-package tests |
| Operator, Graph and outbound Call Loop; exact run context, fuel, delay and integer measure | [Loop contract](docs/spec/loop.md); guarded/timed joins, lifecycle-dependent bodies and broader proof fragments refuse by name |
| Loop exhaustion on an existing host terminal, no contributed port | [v17 amendment](docs/architecture/loop-host-terminal-v17.md); non-Loop identity witnesses unchanged |
| Native Retry of a finite outbound Call, prepared request retained, final-only publication | [Operation contract](docs/architecture/retry-c1-algebra.md); statically checked terminal cases and plan-owned idempotence, closure-based backoff, no Retry port/command or new error case |
| Keyed instances, all five lifetimes, bounded Closing, joins and recovery from recorded records | One synchronous Worker; recovery is not a durable storage backend or distributed runtime |
| Explicit Graph or direct-child closure, idempotent per exact instance generation | [Closure ledger](docs/architecture/instance-stop-command.md); no global key lookup, implicit creation or repeated cancellation |
| Process job/service, File read/replace, HttpClient/HttpServer, WebSocketClient/WebSocketServer | Exact Contracts; HTTP/1.1–3 and complete-message Sessions; resident Process [qualification](docs/architecture/process-service.md), not a model-specific protocol or token-streaming SSE |
| Strict logical types and JSON Schema / Avro / Arrow / OpenAPI projections | Unsupported/lossy cases refuse by path; guest-language execution remains deferred |
| Data annotations, exact HttpClient boot references, privacy attestations, redaction, structured logs and individual typed measurement retention | [Observability contract](docs/spec/observability.md); no measurement report, automatic pricing, subset arithmetic, raw-secret payload, legal-compliance certificate, budget enforcement, FX, FOCUS/OpenCost or OTLP exporter |
| `simulate --ui`: recorded transitions, source, mailboxes, instances, Loop dials, Retry attempts, decisions, causal links, assertion evidence and comparisons | [UI contract](docs/spec/ui.md); read-only source-authored scenarios, no `run --ui`, editing or branches; intermediate Execution transitions within an authored `advance` are individually inspectable |
| `new`, readable golden divergence, `run --record` and `replay [--ui]` | [Diagnostic recording](docs/architecture/diagnostic-recording.md); immutable restitution without a Driver, not durable recovery; classified/boot-dependent recording limits remain explicit |

The [example catalogue](docs/examples/README.md) distinguishes passing programs,
isolated illustrative fragments and historical future-surface sketches.
The [9 September documentation audit](docs/architecture/documentation-audit-2026-09-09.md)
is historical evidence. The [current identity ledger](docs/architecture/connector-rework-identity.md)
and [opening checklist](docs/architecture/opening-showcase.md) distinguish local
qualification from remaining external evidence.

Historical milestone reports retain their original counts and format versions.
[R9](docs/architecture/rework-r9-qualification.md) and
[v17](docs/architecture/loop-host-terminal-v17.md) record separate, declared
format movements; neither rewrites the earlier evidence. The
[release inventory](docs/architecture/release-qualification.md) is the authority
for independent interop and outstanding browser, ngtcp2 and Autobahn work.
Workspace tests do not replace those external qualifications.

Durable storage, multi-Worker/cluster deployment, a complete resident AI gateway,
Timer/Calendar/Memory, Saga, general guests, experiments and broad verification
remain future gates. The [roadmap](ROADMAP.md) orders evidence, not implied
production guarantees.

This white paper is designed to outlive releases. Intent can lead the
implementation, but it must never be confused with what a current binary accepts.

---

## 11. Maxims

A short list of phrases, useful when in doubt. They are mnemonics, not
norms — each compresses a rule whose exact contract lives in
[docs/spec/](docs/spec/); when a maxim and a spec disagree, the spec
wins.

The durability, cluster, Timer/Calendar, experiment and theorem maxims describe
the target architecture; the status table above bounds the shipped subset.

1. **Pure logic belongs in Operators. Effects belong in Connectors.**
2. **An Executor consumes a typed fact and commits state, ordered emissions
   and commands atomically. A Graph scopes and wires; it does not fire.**
3. **Topology is static. Dispatch and closed case routing are dynamic only
   inside the statically enumerated possibilities.**
4. **Every mutable cell has one Executor owner. Key selection addresses a
   logical instance; lifetime closes it; lexical scope exposes immutable
   context. Do not fuse the three.**
5. **Cycles do not appear in the graph. They appear in state, in
   events, and in sequence.**
6. **Operators do not read a hidden clock. `now()` observes one injected,
   journaled transaction instant; Timer and Calendar create explicit time
   events.**
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
14. **Every logical Node owner starts lazily and may enter bounded Closing;
    only Executor instances run delivery-mailbox transactions, only Operators
    author `on stop`, and Graph lifecycle remains a controller fact. A closed
    address may later acquire a fresh generation.**
15. **Two keywords, scope by position: `val` for immutable context and
    `var` for mutable Operator-local state with read-modify-write per
    transaction. Graph `val`s are lexical; Connectors own Contract state but
    no authored `var`; no child inherits an ancestor `var`.**
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
    owning Node instance. Every productible output has an explicit disposition;
    `nothing` is intentional discard, not a default sink. Engine faults and
    panics are supervision, never envelopes.**
17. **The `data` payload is statically typed by default; `variant` is
    the only dynamic type, narrowed before use. The type tree is
    scalar(parameters+logical) / struct / map / array / enum / union,
    anchored on Arrow. Supported JSON Schema, Avro and Arrow schemas —
    including unions — project into this one algebra without lossy
    fallback. `table` is the relational view of
    `array<struct>`, not a type. Numbers are lossless until narrowed.**
18. **Only Operators have handlers, one per trigger. A single-port `where`
    consumes on false as filter sugar; a join `where` admits without consuming
    on false and demands a bound for content correlation. Match owns branching;
    exactly one Dispatch arm executes in a transaction.**

19. **The Lab is the App's twin, never woven into the deployed graph:
    simulations close graphs, experiments exercise real boundaries,
    challenges search bounded spaces, and future theorems may establish
    proofs.**
20. **A Connector contract names semantics; a Driver supplies one physical
    implementation. Exact ids and roles meet once at binding. Static builds and
    future plugin hosts must feed the same registry, while the Execution remains
    ignorant of concrete Connector contracts and implementations.**
21. **Data policy is orthogonal to logical type. Canonical annotations beside
    the resolved schema govern sensitivity, secrets, measures and exact cost
    without changing ordinary assignability. The causal journal remains the
    measurement authority; M1 retains individual facts without a report,
    automatic pricing or subset arithmetic. Hosted logs are bounded,
    never-block projections; OTLP remains deferred. Missing evidence is never
    silently zero; every measurement retains its explicit id, unit, exact value
    and causal provenance. Valuation and the future Budget Extension are
    separate consumers, not annotation fields.**

These maxims are not arbitrary slogans. Each one resolves a specific
architectural ambiguity, and contributors are expected to know them.
