# Rastok roadmap

This roadmap is organized by evidence gates, not by a speculative
sequence of product releases.

## Done — M0 mechanical proof (exited 2026-08-04)

M0 answered one question: can the central technical loop work end to
end? It can — the evidence is recorded in
[`docs/architecture/m0-exit-report.md`](docs/architecture/m0-exit-report.md).

- Parse and check the smallest useful `.rkg` subset.
- Lower it to an executable plan.
- Execute it with one Worker and an in-memory Execution.
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
- The Connector conformance harness over the outbound Call Driver interface —
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
Public opening is **5 November 2026**.

### Connector reorganization — 22 October

The [T5/T9 qualification](docs/architecture/connector-rework-qualification.md)
records the shared standard definitions, per-Driver interfaces, hosted Driver
modules and deterministic per-Node implementation selection. The contributor
path is [catalogue → File → conformance](docs/architecture/connector-drivers.md).
Lab models remain substitutions, not selectable physical providers. The separate
[Process identity movement](docs/architecture/connector-rework-identity.md)
is approved separately by Yoann: `std/process@6`, package format 22,
diagnostic recording 4. Both modes retain their interaction and lifecycle;
the identity inventory tracks their measured witness changes. This update changes no
opening date and authorizes no public operation.

### Human-review follow-up — 5 October

The review's written valve has applied: **24 October** is the real freeze.
The [consolidated human-review ledger](docs/architecture/human-review-m1.md)
owns the three lots: vocabulary without changed bytes, structure without
changed bytes, then the single v20 identity cutover. Vocabulary stops for
Fable's review; the cutover needs Fable's review and Yoann's approval before
reference update. Pre-public registries compact once; the public extend-only contract
starts after that reference update. Keep the chantier numbered v20 for now, and record
the switch to public format 1 as a pre-publication TODO. The approved cutover
used package 20 and recording 4; current formats are named in
[identity.md](docs/spec/identity.md). No publication is authorized here.

### Node realignment close-out — 15 October

The Node-ontology realignment qualified **package 21 / diagnostic recording 4**.
Its [identity inventory](docs/architecture/node-realignment-identity.md) records
the package-only movement: one Node table, Graph-owned wiring and
Operator-owned handlers. Execution indexes do not enter journal identities;
semantic hashes and causal traces remain unchanged. The
[lot 1 qualification](docs/architecture/node-realignment-qualification.md#lot-1-closure--14-october-2026)
records its closure: 1,591 passing tests in 61 groups, no failures or ignored
tests, and the two fixed embedded workloads within the unchanged 128 KiB
heap and 1 MiB code limits.
The depth question is decided by D-2026-10-14-1: format 21 has a 256-level
reader budget, independent of the unchanged 128-level expression and type
bounds. The final gate passes the exact source/transport boundaries and
measures 79,168 B of embedded decoder stack for the accepted level-255
combined package.

The [lot 2 qualification](docs/architecture/node-runtime-qualification.md)
closes inspection-module separation and the shared hosted Worker: public
`run`, `run_until_idle` and `step`, with unchanged execution semantics and
artifacts. Its final gate passes 1,598 Rust tests in 61 groups, 35 browser
tests and 32 other JavaScript tests; the three `no_std` targets and both
embedded workloads pass with unchanged heap and code measurements. Neither
this local qualification nor the browser suite closes the independent opening
gates in the [release inventory](docs/architecture/release-qualification.md).
No public push, tag or publication is authorized by this closure.

### Dated close-out — 7 October

The checkout then used **v20 / diagnostic recording v4**. The
[human-review identity inventory](docs/architecture/human-review-v20-identity.md)
records that reviewed single reference update and qualification. The
[C4 ledger](docs/architecture/retry-c4-qualification.md) is dated evidence
for the preceding native-Retry base. The executable scope includes native Retry,
typed Responses deserialization and generic `@measure(id, unit)` samples retained
on original attempts. The six-chapter gallery, editor grammar and publication
assets are migrated. OTLP export and the Budget Extension remain outside M1;
measurements do not introduce reporting, aggregation or charging authority.

M1's implementation and external opening gates are tracked separately in
[opening-showcase.md](docs/architecture/opening-showcase.md). Independent
browser review, two unaided quickstart trials, Linux/remote release artifacts,
availability of the confirmed public documentation URL and the authorized
publication are not inferred from local tests. The confirmed URLs and site
handoff are recorded in the [release inventory](docs/architecture/release-qualification.md#public-destinations-and-opening-gate).
No public push, tag, registry publication or paid model call is
made by that pass. The embedded Retry workload then left only **512 B of heap**
and **7,136 B of code** margin; the unchanged limits remain gates. M2 must
reclaim bytes before growing the core on this workload.

### Dated implementation checkpoints — superseded by the current ledger

The following records the migration sequence, including earlier candidate
failures and footprint measurements; it is not the current completion status.
R0–R9 and the v17 host-terminal amendment had passed. The 10 September approved exact-instance closure was the
named package exception to the opening freeze: v18, D-2026-09-10-1.
The subsequently requested resident Process introduced a distinct `process@3`
Session contract inside that unchanged package (D-2026-09-14-1); jobs stayed
`process@2`. See the [service ledger](docs/architecture/process-service.md).
The minimal optional-member expression `has(object.member)` was separately
approved by D-2026-09-14-3, with no package-version or journal change.
The Loop / Retry / Saga separation was then approved by D-2026-09-14-4:
one recurrence engine, distinct state-iteration and protected-Call protocols,
and no global Saga effect classification for ordinary Calls. At that checkpoint,
`rework/retry` carried the not-yet-approved v19/recording-v3 candidate; v18 was the
last qualified baseline.
Its [contract and staged gates](docs/architecture/retry-extension-refactor.md)
required an identity inventory before any executable format cutover.
The 15 September C1 design handoff supplied the operation algebra, crash
traces and the v19/v3 inventory; stopping during Retry backoff publishes
no new output (D-2026-09-15-2). Final C4 migration/qualification was still open.
The [C3 checkpoint](docs/architecture/retry-c3-implementation.md) began executing
retained attempts and original-prefix recovery through the staged plan, Lab
and diagnostic recorder. Closing drained unopened requests once without Retry
(D-2026-09-15-4). Its original source gate is superseded by the C4 checkpoint below.
The 16 September qualification follow-up removed implicit Loop-before-Retry
priority at equal deadlines and reproduced the same records after recovery
alongside an ordinary Call timeout. The subsequent shared-admission and
collection-construction pass brought Retry/full recovery to 1,040,824 B,
7,752 B below the unchanged 1 MiB code gate. Its measured heap remained
130,560 B, only 512 B below 128 KiB; metadata growth requires new measurement.
No gate or golden changed.
The subsequent D-2026-09-15-5 approved flat annotation-capable types,
`type Name = @struct { ... }`, using the existing value algebra and explicit
stdlib/runtime interpretations. The [annotation integration handoff](docs/architecture/annotation-types-m1.md)
separates flat declarations from annotation bindings, exact business
measurements and type-directed response deserialization (D-2026-09-15-6).
The first source slice checked flat data with unchanged structural identity.
D-2026-09-16-5 then retained declared descriptive annotation applications, their
nominal definition and ordinary typed constants in candidate tag 6. Aliases
and omitted nullable arguments normalized identically; forged semantic bindings
refused. The next quantity profile implemented the protected measure binding:
simple ids, explicit units and original signed integer/decimal samples.
D-2026-09-18-1 subsequently unified amounts and other quantities under exactly
`id` and `unit`, with signed original-attempt refunds and no monetary profile.
The stdlib schema became executable; its identity migration and subsequent
qualification are recorded in the annotation ledger.
D-2026-09-16-7 removed reports, automatic pricing and generic subset arithmetic
from M1; it did not remove retention or Retry qualification, close A4 or
save reference traces for candidate v19.
D-2026-09-16-1 bound `output out: Response` on HttpClient's JSON
profile, with exact field deserialization and runtime `DecodeFailed` on incompatible
bodies; other codec refinements remained unsupported. The existing annotations
could therefore observe both held and published typed attempts. That deserialization
change allocated no format field; descriptive applications added only the
inventoried candidate annotation tuple. No golden was updated at this step. Its
Retry image remained at 130,560 B peak heap. After named monetary binding and
the simplified definition-table walk, it used 1,048,344 B of code, leaving
only 232 B under 1 MiB. This fixed unannotated workload measures decoder and
validator growth, not maximum-size annotation inspection; further metadata work
must be remeasured.
The 17 September [Retry follow-up](docs/architecture/retry-c3-implementation.md#8-mixed-timers-bounded-publication-and-exact-instances--17-september)
qualified shared deadlines with timed joins, lifetimes and Closing, bounded
fan-out publication, keyed same-origin operations and stop-at-held-batch cuts.
It corrected recovered lifecycle work overtaking the unfinished dated frontier.
The unchanged Retry/full-recovery workload then measured 1,048,368 B of code,
leaving **208 B**, and the same 130,560 B peak heap. Source opening, migration
and the reviewed v19/v3 reference update were still separate gates. That full run had
1,509 passing tests and the same nine not-yet-approved candidate failures; two earlier
cold-start incidents and their passing reruns remain recorded in the report.
The [C4 inspection checkpoint](docs/architecture/retry-c4-inspection.md) opened
the qualified Call profile through public compilation, with source refusals
preserved for unsupported profiles. Runtime tests stopped bypassing that gate.
The debugger began projecting exact operations/attempts, held/final dispositions
and deadlines; a held batch no longer animated a fictitious delivery to App.
Gallery and annotation migration, the one v19/v3 reference update and browser/release
qualification were not closed by that source opening.
The [projected JSON follow-up](docs/architecture/openai-responses-hydration.md)
implemented the approved nullable/additional-member profile without changing
ordinary structs, default deserialization or the plan version. The Responses gateway
then had six executable Lab scenarios; local physical HTTP tests proved original
samples before Retry disposition. No paid OpenAI call was claimed. On the same
Retry/recovery package, code was **1,048,520 B**, leaving **56 B** below 1 MiB;
heap remained **130,560 B**, with 512 B of margin. These were workload-specific
measurements, not permission for further unmeasured core growth. Goldens,
complete annotation migration and the release gates were still open.
The subsequent [explicit-unit migration](docs/architecture/annotation-types-m1.md#16-explicit-unit-migration-inventory--17-september-before-implementation)
removed the implicit measurement registry and retired its compact annotation
tag by name. Quantity ids no longer carried unit, carrier or subset authority:
units became explicit and observations retained their exact signed values.
The same embedded Retry package used **1,046,556 B** of code, recovering
1,964 B and leaving **2,020 B** of margin; heap remained **130,560 B**.
Final stdlib integration, A4 and the common reference update were still open then;
the 18 September [C4 ledger](docs/architecture/retry-c4-qualification.md) records
their closure. Other semantics stayed frozen. The current remaining work is
listed only in the [opening ledger](docs/architecture/opening-showcase.md).

### Opening gates and original slice inventory

The 10 September opening brief separates the gates. The six-example gallery,
measured quickstart, two CLI helpers, release binaries, editor grammar, captures
and GIF, debugger browser pass, CHANGELOG and release preparation block the
opening. Independent HTTP/3-server ngtcp2, official Autobahn and marginal
Connector implementation-cost measurements remain **open but non-blocking**;
their absence stays visible in the release inventory and README. No public
push, release tag or crate publication occurs before 5 November or without
Yoann's instruction. Public documentation is authored here as plain Markdown
in `docs/site/`; the separate site consumes it and the examples at one tag.

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
  `ContractCatalog` describes semantics while runtime `DriverRegistry` describes
  available implementations; these are separate responsibilities, not two
  semantic authorities,
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
  `@sensitive`/`@secret`/`@personal`/`@measure` side-tables, late-resolved secrets,
  structured observations and exact measurement-bearing journal facts.
  Its original closed registry uses OpenTelemetry names;
  D-2026-09-16-3 supersedes that target naming policy with simple author-chosen
  ids, without mandatory prefixes. The [annotation integration](docs/architecture/annotation-types-m1.md#9-simple-names-replace-standards-first-naming-16-september)
  records the executable migration. D-2026-09-16-4 removes the separate
  `@cost` spelling; D-2026-09-18-1 reduces `@measure` to `id` and `unit`
  for every numerical quantity, including amounts.
  The future `Budget` Extension is explicitly outside M1 (D-2026-09-18-2),
  with App as its proposed owner; host and account scope are not yet frozen.
  FX, FOCUS and OpenCost also remain later work. Ordered work and
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
  OpenTelemetry transport rather than fabricating timestamps. A7 originally
  delivered a CostReport; D-2026-09-16-7 supersedes that scope and removes it
  from M1. Original values, schema metadata, provenance and the existing
  run-header attestations remain; no automatic pricing or subset arithmetic
  remains. A8 closes the
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
  redacted DATA/META are projections of existing authorities.
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
identified a missing runtime producer in bounded-join output-productibility
analysis. The opening correction covers both `within` and `last`, independently
in the checker and core, without changing the runtime, plan format or goldens.

## Later — incident lab and safe dogfood

- Introduce the Rust `LogicalTime(i64)` value type with explicit nanosecond
  conversions as part of the relative-timeline refactor in
  [D-2026-10-10-1](docs/spec/decisions.md#d-2026-10-10-1--the-logical-timeline-is-relative-civil-time-is-a-retained-external-input)
  (A-T1-12). Yoann confirmed on 15 October that it belongs with the separation
  of logical and civil time, not in the current runtime-organisation lot.
  It distinguishes execution instants from durations and civil timestamps;
  it adds no DSL type.
- Qualify derived Retry journal indexes before M2 durable storage: current
  prefix scans can make long histories quadratic in total work. Preserve the
  retained arm order and exact recovery facts; the
  [scalability debt](docs/architecture/release-qualification.md#retry-journal-scan-debt-before-m2)
  is not a reason to change M1 semantics or its journal format.
- Design and qualify the physical parallelism profiles in
  [parallelism-notes.md](docs/architecture/parallelism-notes.md): bounded
  multicore Workers, task granularity, and optional flow grouping before
  multiprocess placement. None changes per-instance transaction ownership or
  blocks the M1 local chat's existing I/O concurrency.
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
