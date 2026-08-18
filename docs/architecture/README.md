# Architecture documentation

This directory contains implementation views derived from the normative
specification. It answers *how Rastok is built*; it does not define new
language or runtime behavior.

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
- [`build-order.md`](build-order.md) for the dependency-driven stage
  sequence used to build the M0 modules;
- [`components.md`](components.md) for the component blueprint: owned
  data forms, interfaces, dependency direction, and current status;
- [`spec-to-code.md`](spec-to-code.md) for the map from normative
  documents to implementation responsibilities;
- [`plan-package.md`](plan-package.md) for published v7, the next categorical
  owner-tree target, and the validation boundary;
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
