# Architecture documentation

This directory contains implementation views derived from the normative
specification. It answers *how Rastok is built*; it does not define new
language or runtime behavior.

The [format registry](../spec/identity.md) names the current package and
recording formats. The checkout includes [public Retry admission](retry-c4-inspection.md)
and generic annotated measurements. The [Node realignment inventory](node-realignment-identity.md)
records the Node-table representation. The [Connector identity inventory](connector-rework-identity.md)
records the current single Process Contract, and its
[qualification report](connector-rework-qualification.md) records the gates.
Earlier migration evidence is recorded in
[R9 qualification](rework-r9-qualification.md) for the v16 rework and the
[v17 host-terminal amendment](loop-host-terminal-v17.md), followed by
[v18 exact-instance closure](instance-stop-command.md). Core/HIR share Dispatch and
routing with independent expression, scope, composition and Contract validation.
Operator, Graph and outbound Call Loops execute and recover within the
[named proof subset](../spec/loop.md). Original deadlines, run contexts and
committed deliveries remain journal-owned. Earlier R0–R8 reports are dated
checkpoints, not the current completion status.

Start with the root [`ARCHITECTURE.md`](../../ARCHITECTURE.md), then use:

- [`parallelism-notes.md`](parallelism-notes.md) for the one-Worker baseline,
  future cooperative/multicore/multiprocess profiles, and independent flows as
  placement candidates rather than new Executors or mandatory tasks;
- [`annotation-types-m1.md`](annotation-types-m1.md) for the approved `@struct`
  syntax, protected stdlib/runtime metadata boundary,
  business-measure/typed-deserialization migration, dated checkpoints and current
  two-field measurement contract;
- [`retry-extension-refactor.md`](retry-extension-refactor.md) for the approved
  Loop/Retry separation and C0–C4 gates; C1's [algebra](retry-c1-algebra.md),
  [reference traces](retry-c1-traces.md) and [identity inventory](retry-c1-identity.md)
  are the 15 September design handoff, not executable v18 support;
- [`instance-stop-command.md`](instance-stop-command.md) for the accepted
  explicit-lifecycle command implementation, child-only closure and v18 ledger;
- [`process-service.md`](process-service.md) for the resident Process Contract,
  real/Lab transport, shutdown, dialogue and remaining gateway limits;
- [`agent-gateway-showcase.md`](agent-gateway-showcase.md) for the composed
  resident WebSocket bridge, its readable HTTP/vanilla-JS chat continuation,
  ownership/order regressions and native JSON protocol seam;
- [`openai-responses-hydration.md`](openai-responses-hydration.md) for the
  confirmed Responses target, official schema contact and the approved limited
  executable JSON projection profile and HTTP adapter qualification;
- [`process-contract-review.md`](process-contract-review.md) for the requested
  historical Process job/service design review and its subsequent amendments;
- [`documentation-audit-2026-09-09.md`](documentation-audit-2026-09-09.md) for
  the dated whitepaper/specification/example/UML audit, executable checks
  and the subsequently corrected join-productibility discrepancy;
- [`m0-slice.md`](m0-slice.md) for the first end-to-end proof;
- [`m0.5-slice.md`](m0.5-slice.md) for the kernel/package consolidation;
- [`m1-slice.md`](m1-slice.md) for the current public-opening milestone
  and its landed/pending inventory;
- [`m1.3-type-system-backlog.md`](m1.3-type-system-backlog.md) for the
  ordered landing of logical payload types, uniform ports and tagged
  frontends;
- [`m1.4-instances-backlog.md`](m1.4-instances-backlog.md) for the ordered
  rebase onto Node-owned keys, lifetime, hierarchical addresses and bounded
  Closing;
- [`m1.4-exit-report.md`](m1.4-exit-report.md) for the R7 negative-space,
  identity-ledger, portability, and placement-neutrality evidence;
- [`m1.5-connectors-backlog.md`](m1.5-connectors-backlog.md) for the ordered
  landing of stable Connector contracts, role-based Drivers, static/runtime
  binding and the first two-Contract proof with `File`;
- [`m1.5-exit-report.md`](m1.5-exit-report.md) for the C6 architecture,
  identity ledger, plugin non-claims and remaining Connector limits;
- [`m1.6-http-websocket-backlog.md`](m1.6-http-websocket-backlog.md) for the
  ordered Event/Call/Session foundation, HTTP/1.1-3 Drivers and
  WebSocket Codex gateway gates;
- [`m1.6-exit-report.md`](m1.6-exit-report.md) for H7 static assembly,
  interoperability, identity, binary-size, no-std and QEMU closure evidence;
- [`m1.7-loop-backlog.md`](m1.7-loop-backlog.md) for the ordered Node taxonomy,
  Loop contract, categorical transport, journal fold, carrier execution and
  closure gates;
- [`m1.7-exit-report.md`](m1.7-exit-report.md) for the public composition,
  three-register identity, portability and Cortex-M33 closure evidence;
- [`m1.8-observability-backlog.md`](m1.8-observability-backlog.md) for the
  ordered data-policy, secret, observation and exact-cost foundation;
- [`m1.8-exit-report.md`](m1.8-exit-report.md) for the policy, accounting,
  identity, portability and interoperability closure evidence;
- [`rework-r0-algebra.md`](rework-r0-algebra.md),
  [`rework-r0-reference-traces.md`](rework-r0-reference-traces.md),
  [`rework-r0-repository-audit.md`](rework-r0-repository-audit.md), and
  [`rework-r0-identity.md`](rework-r0-identity.md) for the frozen v16 algebra,
  hand-written host traces, repository claims and identity movement ledger;
- [`rework-r1-host-contracts.md`](rework-r1-host-contracts.md) for the exact
  Operator, Graph and outbound-Call Loop profiles and six versioned Contract
  `@2` sheets, now exercised by the R8/R9 host matrix; the M1.7 files remain
  historical evidence for v14/v15;
- [`rework-r2-supersession.md`](rework-r2-supersession.md) for the historical
  R2 alignment matrix, then-current-v15/target-v16 boundary, repository-contact
  corrections and review stop before R3 (now completed);
- [`release-qualification.md`](release-qualification.md) for the single
  inventory of executed third-party interoperability evidence and external
  qualifications still required before a public release;
- [`connector-drivers.md`](connector-drivers.md) for the File-based contribution
  path: catalogue, Contract, required interfaces, implementation selection, Lab
  substitution and conformance;
- [`connector-rework-qualification.md`](connector-rework-qualification.md) for
  the T5/T9 module, implementation-selection and catalogue checks;
- [`connector-rework-identity.md`](connector-rework-identity.md) for the separate
  proposed Process Contract identity change, which requires explicit approval;
- [`build-order.md`](build-order.md) for the dependency-driven stage
  sequence used to build the M0 modules;
- [`components.md`](components.md) for the component blueprint: owned
  data forms, interfaces, dependency direction, and current status;
- [`spec-to-code.md`](spec-to-code.md) for the map from normative
  documents to implementation responsibilities;
- [`plan-package.md`](plan-package.md) for the categorical package and
  independent validation boundary, with earlier format ledgers marked historical;
- [`node-realignment-qualification.md`](node-realignment-qualification.md) for
  the boundary-face invariant, measured Node-table layout and lot 1 closure;
- [`node-runtime-qualification.md`](node-runtime-qualification.md) for lot 2:
  separate read-only inspection and the shared hosted Worker;
- [`parallelism-notes.md`](parallelism-notes.md) for the keyed-mailbox
  concurrency model and the decisions left for physical parallelism;
- [`spike-rp2350-emulated.md`](spike-rp2350-emulated.md) for the measured
  `no_std` kernel proof;
- `01-system-context` for the system and its users;
- `02-rastok-container` for executable and deployment boundaries;
- `03-rastok-components` for current logical responsibilities inside the
  executable;
- `04-app-lab-effect-cycle` for the main runtime interaction;
- `05-rastok-packages` for the current core data types: envelopes, instances,
  the canonical registry, records and unsealed deadlines;
- `06-rastok-modules-overview` for the current Rust module inventory and
  one-way crate dependency;
- `07-node-executor-extension` for semantic ownership: Graph, Executor,
  Operator, Connector/Contract, Extension and its manager. These categories
  do not imply a public Rust trait for every box.

## Diagram conventions

- `.puml` files are editable sources; adjacent `.svg` files are generated
  review artifacts.
- Diagrams are descriptive, not normative. When a diagram and
  `docs/spec/` disagree, the specification wins.
- C4 containers represent executable or deployable units. Rust modules
  are not promoted to containers merely to make a diagram look full.
- Component boxes describe responsibilities. They do not pre-allocate
  crates or public APIs.
- Dynamic behavior that is clearer as an interaction is shown as a
  sequence rather than forced into a static C4 view.
- Includes come from PlantUML's bundled standard library. Diagram builds
  do not fetch remote templates.

Render every diagram from the repository root:

```sh
tools/diagrams/render.sh
```

The initial views were checked with PlantUML 1.2026.6. The source should
remain compatible with the bundled C4 standard library rather than rely
on a machine-specific include path.

The 22 October Connector series updates views 03, 04 and 06 and regenerates
their SVGs from the reviewed sources. They show the shared Contract definitions,
implementation selection and execution-owned polling; no format change follows
from these module names.

The 7 October documentation sweep refreshes views 03–07 against the current
format registry and regenerates their SVG with the installed PlantUML renderer.
View 05 now describes implemented types rather than a pre-build design. View 04
remains explicitly an outbound **Call**, not the resident Session frame protocol.
No measured report, OTLP implementation, Budget manager or plugin loader is
implied by these views; no renderer or frontend dependency is added.
