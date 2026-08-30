# Rastok roadmap

This roadmap is organized by evidence gates, not by a speculative
sequence of product releases.

## Done — M0 mechanical proof (exited 2026-08-04)

M0 answered one question: can the central technical loop work end to
end? It can — the evidence is recorded in
[`docs/architecture/m0-exit-report.md`](docs/architecture/m0-exit-report.md).

- Parse and check the smallest useful `.rkg` subset.
- Lower it to an executable plan.
- Execute it in a one-worker in-memory VM.
- Run one scenario with a mock, virtual time, and a golden trace.
- Cross one real `Process` boundary.
- Produce a useful diff at the first divergence.
- Reproduce the result on two machines.

Out of scope: a complete connector catalogue, live Saga or Loop
managers, durable deployment, enterprise security, UI, registry, and
cluster operation.

## Done — M0.5 engine consolidation (closed 2026-08-06)

Consolidated what M0 proved before widening it
([`docs/architecture/m0.5-slice.md`](docs/architecture/m0.5-slice.md)):

- The delivery reference specified and the call identity derived
  from it — the multiport prerequisite, spec-first.
- The no_std-ready core (`rastok-core`) split from the std layer,
  with CI guarding the frontier; neutrality proven by the unchanged
  semantic hash.
- The executable plan carried as a versioned, verifiable package
  with typed refusals.
- The embedded spike run emulated against its written go/no-go —
  provisional GO, silicon confirmation pending hardware.
- The connector conformance harness over the outbound Connector Driver seam —
  the M0 subset of the ten contract cases made executable (error-set,
  journal, timeout, cancel, silence, and interleaved-correlation
  disciplines; the declarative cases as sheet checks), with every
  unproven gap explicit; `Process` re-expressed on it.
- The verified-authorship benchmark built and rehearsed dry — twenty
  referee-checked tasks, a frozen prompt card, the full
  generate → check → retry → simulate loop measured on recorded
  attempts; the paid model wave is deliberately deferred.

## Now — M1, the demonstrable integration slice

The question this slice answers: can a realistic asynchronous
integration seam — request, external partner, late callback — be
built, simulated, and shown end to end, before its real systems
exist? Scope, order, and evidence:
[`docs/architecture/m1-slice.md`](docs/architecture/m1-slice.md).
The repository opens publicly with this milestone (15 October 2026).

- Multiport execution, then the constructs that stand on it: Node-owned
  logical instances (`instance.key`), orthogonal lifetime and bounded
  Closing of open invocations, followed by bounded joins.
- A JSON/JSONL codec chain across the boundaries.
- M1.3 resets the payload foundation: one Rastok-owned logical type and
  value algebra, complete effective types on every port, unions across
  JSON/Avro/Arrow projections, one expression consumer, and ordinary
  multi-output completions (for `Process`: decoded `out`, exact stderr on
  `err`, and one typed `exit` per invocation). This subsumes the original
  typed-error item: its codes become `ProcessExit` cases. Node-instance
  `stop` remains a distinct, later lifecycle boundary.
  B1's complete strict-profile type/value domain, B2's total validated
  port-type resolution, B3's Rastok-owned expression consumer/narrowing
  core, B4's one `name!` frontend surface, and B5's uniform resolved
  Connector port contracts are landed. B6 has added general tagged-value
  construction, one contextual union-wrapping door and ordinary `err`
  emission. B7 has replaced the success/failure seam with atomic ordered
  completions: `Process(job)` now publishes unit-framed `out*`, independently
  framed stderr on `err*`, and exactly one typed `exit`. B8 has published the
  categorical v7 type/value/expression registry and mandatory total resolved
  port table, removing the last Process/decimal transport bridges. B9 has
  closed M1.3: strict JSON Schema, Avro and Arrow projections either preserve
  the complete logical type or refuse by path and reason; `variant` requires
  explicit narrowing before structural inspection; the obsolete v6
  type/expression scaffolding is gone; and the cross-layer corpus, three
  no-std targets and emulated Cortex-M33 proof are green. The closing review
  also removed the last synthetic Operator fault channel: an optional
  `err: E` is explicit domain output, while engine faults abort and reach
  supervision.
- M1.4 has been normatively rebased before public release. The unreleased
  P0–P3 named-`partition` / `per` implementation is superseded as a source and
  transport model, while its canonical-key, pre-commit routing, mailbox,
  cursor and lifecycle proofs remain reusable. D-2026-08-18-1 replaces it
  with Node-owned `instance.key`, independent `lifetime`, Node-private
  `var`, hierarchical logical addresses, and independently shardable keyed
  boundaries. The ordered R0–R7 gates in the
  [M1.4 instance backlog](docs/architecture/m1.4-instances-backlog.md) now
  realign compiler/transport/runtime through public composition and joins. M1
  keeps the synchronous one-worker reference engine; the same addresses are
  designed for future concurrent Workers and Coordinator/child-Worker
  deployment without placing physical facts in causal identity. Its first R2
  checkpoint allocated categorical plan-package v9: lexical Graph/Node
  owners, scoped edges, typed key/lifetime policy, phased immutable values and
  dynamic Connector rows now travel and are revalidated independently of the
  compiler ContractCatalog. R3 landed hierarchical routing: canonical
  `(address, generation)` references are sealed by producers and shared by
  journal, mailboxes, state, calls, timers, lifecycle and cursors; Graph keys
  are sticky across descendant traffic; candidate bindings fail before
  ingress/effect; and a stable virtual-shard seam proves placement neutrality.
  This deliberately moved causal traces, including singleton traces, without
  moving semantic hashes or plan-package bodies. R4 executes all five lifetime
  profiles with journal-derived generations, deterministic deadlines,
  parent/child lifecycle fencing, and inner-first natural closure. R5 now gives
  every finite effectful instance one explicit bounded Closing contract:
  durable close admission, drain, canonical cancel, settlement to an observed
  exit or `OutcomeUnknown`, descendant closure, `on stop`, reclamation, then
  successor-generation admission. R6 closes the public nested-Graph and Lab
  path and executes joins in the owning Operator instance, with FIFO
  candidate selection, `within`/`last` eviction, typed `JoinExpired`, lazy
  named inputs, and static CommonOrAbsent presence checks. Existing program
  identity and causal goldens remain fixed. R7 closes the executable M1.4
  slice: removed forms are recovery-only, the complete identity-move ledger
  is audited, and moving one stable virtual shard between physical Workers is
  proved not to change semantic hash, journal bytes, or replay input. The
  one-worker reference remains the only runtime claimed; cluster leases and
  recovery are later work.
- M1.5 is closed: contracts have stable versioned identities, compiler
  `ContractCatalog` and runtime `DriverRegistry` become separate authorities,
  and one pre-journal binder serves mixed connector families through explicit
  inbound/outbound Driver roles. Static composition is first-class and keeps
  the same registry boundary available to future plugin hosts. `File` is the
  proof connector: one-file `read` and atomic `replace`, with directories and
  `watch` deliberately outside this slice. See the
  [ordered M1.5 backlog](docs/architecture/m1.5-connectors-backlog.md).
  C0's pure Driver-seam refactor, C1's categorical v10 identity/role
  transport, C2's mixed-family pre-journal binder, and C3's Registry-free
  `rastok/file@1` sheet plus pre-intent invocation settings are landed. C4's
  real one-file Driver now performs bounded complete reads and native atomic
  replacement only after OUTBOUND open, with portable exits, cooperative
  cancellation and no-recall replay. C5 now runs Process and File through one
  role-aware conformance battery, binds multi-family Lab mocks through the same
  categorical registry, proves real/Lab plan identity and Process-only,
  File-only and combined static builds. C6 closes the slice with a data-only
  capability manifest, one documented Connector contribution path, and a
  repository-wide vocabulary/diagram audit. It freezes no serialized plugin
  manifest syntax or native plugin ABI. The evidence and identity ledger are
  in the [M1.5 exit report](docs/architecture/m1.5-exit-report.md).
- M1.6 closes the hosted network boundary with four contracts:
  `HttpClient`, `HttpServer`, `WebSocketClient`, and `WebSocketServer`.
  HTTP/1.1, HTTP/2 and HTTP/3 are physical bindings of the same HTTP
  contracts; WebSocket is a long-lived session with many messages and one
  typed terminal `exit`, never one fake invocation per frame. The ordered
  [M1.6 backlog](docs/architecture/m1.6-http-websocket-backlog.md) has landed
  H1's generic `Event | Call | Session` lifecycle, placement-neutral session
  references and two-phase inbound admission. H2 publishes the exact four
  network sheets, lossless HTTP metadata, codec-selected ports, portable exit
  unions, Registry-free mirrors and family-owned Lab bindings. H3 now pins one
  bounded hosted pump plus Hyper/Rustls for HTTP/1.1-2, quiche/BoringSSL for
  HTTP/3, and Tungstenite, with a real-UDP
  [h3 + Quinn comparison](spikes/h3-quinn-http3/README.md), one shared trust
  corpus and no async/core leakage. H4 now executes the same HttpClient sheet
  over all three versions, with pre-open exact requests, visibility-safe
  fallback, bounded bodies, lossless response metadata, outbound
  backpressure and the generic Call conformance battery. H5's generic inbound
  Call seam is now landed: bounded commit-before-accept opens the Call on its
  exact source instance, and one typed `l:reply` transaction closes it without
  an OUTBOUND record. H5.1 now executes `ack` through a live bounded Hyper
  HTTP/1.1–2 listener: body memory remains charged through admission, mailbox
  pressure withholds 202, and both versions close the same Call shape. H5.2
  adds the generic direct-lineage response return: the graph seals the exact
  opening instance before commit, bounded Driver admission may remain pending,
  and the terminal consumes the real response cursor without an OUTBOUND
  record. Final H5 now runs HTTP/1.1–3 through the same bounded retained Call,
  executes `respond`, checks explicit direct-path correlation where roots
  mix, arms bounded request deadlines, closes response/timeout races once,
  and keeps the standalone runner polling without stdin. H6 executes the
  WebSocket Drivers over HTTP/1.1 Upgrade plus HTTP/2 and HTTP/3 Extended
  CONNECT, with one bounded generic Session fold and the executable Codex
  gateway. H7 closes static reduced/full registry assembly, feature and
  no-std matrices, independent interop and QEMU evidence. The full ledger is
  in the [M1.6 exit report](docs/architecture/m1.6-exit-report.md).
  H4.1 has now landed the schema-facing HTTP contract: canonical open
  `media_type`, typed standard request headers plus an open custom-header map,
  typed response metadata, strict OpenAPI 3.0/3.1 projection, and the
  project-wide `--resolve-open-api` generated-source workflow. Its categorical
  v12 package move is isolated from the later H5/H6 physical execution, which
  introduced no further package-format move.
- M1.7 makes `Loop` the first NodeAspect to receive categorical transport and
  then become executable. L0–L7 are landed and the slice is closed:
  `node::{Kind, Type, Aspect}` are separate,
  Reducer/Router remain patterns, source spelling is singular
  `instance = { key = ... }`, and positive `Loop.fuel` plus closed
  `Loop.delay` cross checked HIR and the categorical v14 form retained by v15.
  The exact Loop boundary,
  closed aspect contract, ordinary Graph-owned handlers, and the finite
  progress decision are re-derived Registry-free. Immediate persistent
  root-Graph Loops now execute through a per-root journal fold: exact fuel,
  `l:next`, exhaustion, concurrent runs and crash reconstruction are proven
  without a spatial back-edge. L5 now executes fixed and safely saturating
  exponential `Loop.delay` on the shared dated queue. Timer kind 5 is sealed
  by the `l:next` id; an exactly-once `Admit` transition selects and commits
  bounded spatial routes only when due; direct child output handlers execute
  after their producer commits. Equal-date wakes retain arm order, and
  instance Closing fences every stale generation before its successor. A
  fixed lifetime is proven not to inherit or wait for a later Loop delay, and
  a Process timeout at 1s is proven to precede an independent 2s recurrence
  delay. Direct-body recovery re-arms the persisted annotation; spatial
  recovery remains named until general instance/Connector replay. The temporal
  substrate also exposes `now()` only as one transaction-fixed logical
  timestamp, supplied by the VM and sealed when observed; the hosted runner
  maps wall origin plus monotonic elapsed time, while Lab/bare-metal callers
  own the origin and `advance`.
  The deferred event source is named `Timer`, distinct from that runtime
  timeline and from civil `Calendar`. L6 now admits the first proof-only Loop:
  a direct signed-integer `Loop.measure`, positive on every recurrent direct
  input path and updated as `measure - k`, produces evidence which the
  deterministic `no_std` core re-derives after current decode. Non-trivial
  invariants, computed/lexicographic measures and Connector-dependent progress
  remain named theorem gates. L7 exercises the final public composition
  surface: singleton and keyed concurrent runs, ordinary nested Graphs,
  bounded Process retry with multi-frame data, exhaustion, and
  instance-Closing fences. It
  closes the separate package/semantic/causal identity audits, three no-std
  builds, independent interop and QEMU evidence. Operator work and Connector
  calls remain ordinary children of the Loop Graph; no privileged `err`/`exit`
  spelling determines policy. See the
  [ordered M1.7 backlog](docs/architecture/m1.7-loop-backlog.md) and
  [exit report](docs/architecture/m1.7-exit-report.md).
- M1.8 adds the data-policy, privacy and accounting foundation: canonical
  `@sensitive`/`@secret`/`@personal`/`@measure`/`@cost` side-tables, late-resolved secrets,
  structured observations, and exact per-flow measure/cost reports carried by
  the journal. OpenTelemetry names are used wherever their semantic
  conventions match; `rastok.*` fills only uncovered runtime concepts. Budget
  enforcement, FX, FOCUS and OpenCost remain later consumers. Ordered work and
  the calendar-safe OpenTelemetry cut are in the
  [M1.8 backlog](docs/architecture/m1.8-observability-backlog.md); the explored
  post-M1 reservation model is preserved separately in the
  [Budget draft](docs/architecture/budget-governance-draft.md). A0–A3.1 are
  closed: categorical v15 carries the five contracts as canonical sparse
  side-tables; Registry-free validation derives their effective result through
  canonical operations, and presentation redacts confidential, secret and
  personal data before formatting. Unannotated semantic hashes and causal
  traces remain fixed. Clear secret and personal payloads stay fail-closed
  pending A4's attested deployment boundary.
- A live view of the running graph, fed by the journal.
- A guided example gallery seeded by the benchmark's reference
  solutions, closed by the October demo flow.

## Later — incident lab and safe dogfood

- Turn one real internal failure into a redacted fixture and scenario.
- Run in a trusted, non-critical environment with a kill switch.
- Measure installation time, diagnosis time, cost per session, and
  maintenance effort.
- Test the same proof with selected design partners.

## Then — transferability

- A third party installs Rastok from the documentation.
- That party changes or writes a graph.
- A third-party incident becomes a replayable regression scenario.
- At least one partner uses the proof in its own technical evaluation.

Only evidence from these gates determines the next product scope.
