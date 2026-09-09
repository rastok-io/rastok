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
The original public-opening target was 15 October 2026. The v16 rework now
takes semantic priority; no public date overrides its R0–R9 exit gates.

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
  and one pre-journal binder serves Connectors with different Contracts through explicit
  inbound/outbound Driver roles. Static composition is first-class and keeps
  the same registry boundary available to future plugin hosts. `File` is the
  proof connector: one-file `read` and atomic `replace`, with directories and
  `watch` deliberately outside this slice. See the
  [ordered M1.5 backlog](docs/architecture/m1.5-connectors-backlog.md).
  C0's pure Driver-seam refactor, C1's categorical v10 identity/role
  transport, C2's mixed-Contract pre-journal binder, and C3's Registry-free
  `rastok/file@1` sheet plus pre-intent invocation settings are landed. C4's
  real one-file Driver now performs bounded complete reads and native atomic
  replacement only after OUTBOUND open, with portable exits, cooperative
  cancellation and no-recall replay. C5 now runs Process and File through one
  role-aware conformance battery, binds multi-Contract Lab mocks through the same
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
  unions, Registry-free mirrors and Contract-owned Lab bindings. H3 now pins one
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
- M1.7 is closed as historical v14/v15 evidence. It proved finite temporal
  recursion, exact fuel exhaustion, the first re-derived integer measure,
  logical delay, concurrent runs and Closing fences without a spatial
  back-edge. Its Graph-handler and port-shaped Loop control model is
  superseded by D-2026-09-05-1, not erased from history. See the
  [ordered backlog](docs/architecture/m1.7-loop-backlog.md) and
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
  [Budget draft](docs/architecture/budget-governance-draft.md). M1.8 is
  closed: categorical v15 carries the five contracts as canonical sparse
  side-tables; Registry-free validation derives their effective result through
  canonical operations, and presentation redacts confidential, secret and
  personal data before formatting. Exact boot references now bind real and
  scripted HttpClient settings without admitting secret material into graph or
  causal artifacts. Personal data runs only with an attested run-scoped
  privacy profile; raw incoming secret fields remain fail-closed. Unannotated
  semantic hashes and causal traces remain fixed. Typed record correlations
  are retained beside causal history, and the opt-in JSON exporter is bounded,
  never-block and payload-free by default; saturation changes neither
  execution nor accounting. A6 qualifies and explicitly defers the optional
  OpenTelemetry transport rather than fabricating timestamps. A7 derives one
  public exact per-flow `CostReport` lazily from causal envelopes and resolved
  schemas: inclusive token/subset relations cannot double-count, unknown
  prices remain typed `unpriced`, exact provider and pricebook amounts retain
  basis/currency, and an optional deployment price profile is attested in the
  run header without moving graph or causal identity. A8 closes the
  cross-layer audit, three-register identity ledger, independent interop,
  three-ISA `no_std` builds and Cortex-M33 evidence; see the
  [exit report](docs/architecture/m1.8-exit-report.md).
- The v16 semantic rework has reached R9. Node/Graph/Executor/Connector/Extension,
  explicit destinations, recursive Dispatch, case routing and multi-emission
  execute through one independently validated package. Operator, Graph and
  outbound Call Loops retain exact run contexts and recover committed commands,
  mailboxes, exchanges and original deadlines. The proof subset remains named
  in [loop.md](docs/spec/loop.md); unsupported profiles refuse at check.
  [R9 qualification](docs/architecture/rework-r9-qualification.md) owns the
  single golden move, unchanged gauge/hash/causal witnesses, migration inventory
  and final gates. Earlier R0–R8 reports remain dated checkpoint evidence.
- The [v17 Loop amendment](docs/architecture/loop-host-terminal-v17.md) removes
  the contributed expiry port: exhaustion is a typed case on the existing host
  terminal (`err`, Call `exit`, or an explicit target). The debugger's iteration
  dial follows admitted applications in the displayed journal prefix. Non-Loop
  semantic and causal witnesses remain unchanged; the ledger records the
  categorical package move and qualification separately from R9.
- R9 realigns the visual debugger with this model: `simulate --ui` only,
  initially paused, read-only retained history, source-authored inputs and no
  branch authoring. Reset opens a fresh simulation session; past sessions remain
  inspectable. Real ports, mailbox FIFOs, instances, Loop-run dependencies,
  redacted DATA/META and CostReport are projections of existing authorities.
  Manager commands never create spatial edges. Playback, pan/zoom, inspector
  tabs and splitters remain physical UI state. See [ui.md](docs/spec/ui.md).
  The [explanatory projection pass](docs/architecture/simulation-debugger-explanations.md)
  adds retained decisions, causal links, pending deadlines, assertion evidence
  and instance/session comparisons, without changing execution or plan identity.
  The external browser/keyboard pass remains in the
  [release qualification](docs/architecture/release-qualification.md) inventory.
- A rich terminal debugger remains a later projection over the same controller
  and redacted read model; its CLI spelling is deliberately not frozen in M1.9.
- A guided example gallery seeded by the benchmark's reference
  solutions, closed by the October demo flow.

The [2026-09-09 documentation audit](docs/architecture/documentation-audit-2026-09-09.md)
refreshes the whitepaper, specifications, checked examples and UML views. It also
records an open implementation defect: timed-join `JoinExpired` production is
missing from output-productibility analysis. Its route must remain explicit;
the new expense-approval fixture demonstrates why the warning is unsound.

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
