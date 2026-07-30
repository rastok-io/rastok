# Architecture documentation

This directory contains implementation views derived from the normative
specification. It answers *how Rastok is built*; it does not define new
language or runtime behavior.

Start with the root [`ARCHITECTURE.md`](../../ARCHITECTURE.md), then use:

- [`m0-slice.md`](m0-slice.md) for the first end-to-end proof;
- [`spec-to-code.md`](spec-to-code.md) for the map from normative
  documents to implementation responsibilities;
- `01-system-context` for the system and its users;
- `02-rastok-container` for executable and deployment boundaries;
- `03-rastok-components` for logical responsibilities inside the M0
  executable;
- `04-app-lab-effect-cycle` for the main runtime interaction.

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
