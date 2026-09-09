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

The [2026-09-09 audit](../../docs/architecture/documentation-audit-2026-09-09.md)
regenerates the seven views with PlantUML 1.2026.8. Views 01–04, 06 and 07
describe the current architecture; view 05 deliberately retains its archived
pre-build model. Generated SVG layout may change with renderer versions without
changing that archived source.

For raster review, use a white canvas, matching the SVG background. Some SVG
rasterizers do not paint the root CSS background automatically:

```sh
rsvg-convert --background-color white --width 1400 \
  --output /tmp/rastok-semantic-model.png \
  docs/architecture/07-node-executor-extension.svg
```
