# Architecture documentation

This directory contains implementation views derived from the normative
specification. It answers *how Rastok is built*; it does not define new
language or runtime behavior.

The source/HIR checkpoint is
[`rework-r3-source-hir.md`](rework-r3-source-hir.md). R4 is open at
[`rework-r4-plan.md`](rework-r4-plan.md): the independent closed-case verifier
is wired to both source consumers. Core/HIR share Dispatch and routing, with
independent local expression/scope checks and whole-inventory route conflict
validation. Categorical v16 transport and execution remain pending.

Start with the root [`ARCHITECTURE.md`](../../ARCHITECTURE.md), then use:

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
  `@2` sheets. These are design gates only; R3 is the first implementation
  gate, and the M1.7 files remain historical evidence for v14/v15;
- [`rework-r2-supersession.md`](rework-r2-supersession.md) for the live-doc
  alignment matrix, current-v15/target-v16 boundary, repository-contact
  corrections and the explicit review stop before R3;
- [`release-qualification.md`](release-qualification.md) for the single
  inventory of executed third-party interoperability evidence and external
  qualifications still required before a public release;
- [`connector-drivers.md`](connector-drivers.md) for the current contribution
  path from semantic Contract through real/Lab Driver conformance and static
  composition, plus the deliberately narrow future-plugin seam;
- [`build-order.md`](build-order.md) for the dependency-driven stage
  sequence used to build the M0 modules;
- [`components.md`](components.md) for the component blueprint: owned
  data forms, interfaces, dependency direction, and current status;
- [`spec-to-code.md`](spec-to-code.md) for the map from normative
  documents to implementation responsibilities;
- [`plan-package.md`](plan-package.md) for current categorical v15, its retained
  v9 owner/instance and v7 type/value/expression registries, stable Connector
  identities/roles/exchange lifecycles, H4.1 `media_type` extension, canonical
  schema-policy tables, and the validation boundary; its opening note points
  to the frozen v16 movement ledger without assigning wire tags before R4;
- [`parallelism-notes.md`](parallelism-notes.md) for the keyed-mailbox
  concurrency model and the decisions left for physical parallelism;
- [`spike-rp2350-emulated.md`](spike-rp2350-emulated.md) for the measured
  `no_std` kernel proof;
- `01-system-context` for the system and its users;
- `02-rastok-container` for executable and deployment boundaries;
- `03-rastok-components` for logical responsibilities inside the M0
  executable;
- `04-app-lab-effect-cycle` for the main runtime interaction;
- `05-rastok-packages` for the **archived pre-build M0** Rust module and
  type sketch. It deliberately preserves superseded names and must not
  be read as the current API;
- `06-rastok-modules-overview` for the maintained bird's-eye module
  companion, including the landed M1 compiler interludes and
  module-level dependencies.

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
