# Diagram tooling

Architecture diagrams are written in PlantUML and rendered as SVG next
to their sources in `docs/architecture/`.

From the repository root:

```sh
tools/diagrams/render.sh
```

The script uses `plantuml` from `PATH`, or the executable named by the
`PLANTUML` environment variable. PlantUML's bundled C4 standard library
is used locally; rendering does not fetch remote templates.

Both `.puml` and `.svg` are kept:

- review the `.puml` file for the editable model;
- embed or browse the `.svg` file without requiring PlantUML;
- regenerate all SVGs after changing a source.

The initial render was checked with PlantUML 1.2026.6 and Graphviz
15.1.0.
