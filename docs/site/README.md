# Public documentation content contract

Version: 10 September 2026. This directory belongs to the Rastok repository.
The separate `rastok-site` repository consumes it at one explicit Git tag,
together with `examples/`, the referenced goldens and the editor grammar.
There is no site runtime, Node dependency, MDX or component import here.

## Public destination handoff: 18 September 2026

The confirmed documentation destination is `https://docs.rastok.com`; the
homepage is `https://www.rastok.com`. The source and release destinations are
recorded in the [release inventory](../architecture/release-qualification.md#public-destinations-and-opening-gate).
No front-matter field changes with this handoff.

For the authorized 5 November opening, prefer a minimal Starlight deployment
serving the quickstart and six gallery chapters. If that cannot be ready,
serve a 301 from the documentation domain to the actual gallery page on
`www.rastok.com`, replacing it with the documentation site in November. The
fallback gallery path must be confirmed in `rastok-site`, not invented here.
The bare `rastok.com` domain's 301 target is `https://www.rastok.com`.

This is a deployment requirement for the separate site repository, not a
completed DNS, redirect or deployment change. The documentation URL must
resolve before crates are published; verification remains an opening gate.
No deployment or publication is authorized by these local file changes.

## Page schema

Pages are ordinary UTF-8 Markdown with one leading YAML front-matter block.
This README is the schema, not a page, and has no front-matter. The consumer
must reject missing, unknown or wrongly typed fields, duplicate keys, broken
references and duplicate orders within a section. Empty strings and empty lists do not
meet a required field. No field is optional in the following tables.

The required introductory pages are `index.md` (Overview, order 1) and
`getting-started.md` (Getting Started, order 2). The six runnable example
pages live in `examples/`.

The User Guide lives in `guide/`, between General and Examples in the site
navigation. Its six opening chapters use only `title`, `description` and
`order`, with unique positive orders 1 through 6. No extra front-matter field
is needed for the guide or its diagrams.

Every page has:

| Field | Type | Meaning |
|---|---|---|
| `title` | string | Page title; the site emits the single H1 |
| `description` | string | One-sentence summary for readers, search, JSON-LD and `llms.txt` |
| `order` | positive integer | Navigation order within the containing section |

Every page in `examples/` additionally has:

| Field | Type | Meaning |
|---|---|---|
| `teaches` | string | Concepts demonstrated by the executable example |
| `graph` | string | Repository-root-relative path to an existing `examples/*.rkg` file |
| `golden` | string | Repository-root-relative path to an existing canonical `.trace` file |
| `command` | string | Tested command, run from the repository root |
| `expected` | string | Observable result, not a promise or inferred behavior |
| `break_it` | string | Exact source edit and rerun command that expose a difference |
| `limits` | nonempty list of strings | Limitations visible without reading implementation notes |

References never use absolute paths, `..`, external URLs or symlinks outside
the checkout. The site reads the graph and golden from these fields; do not
paste a whole graph or trace into the Markdown body. `golden` identifies the
representative scenario; `--compare` takes its directory and checks **all**
scenario files named `<simulation>.<scenario>.trace`.

No additional field has been introduced. Any extension is documented here
before use and reported to Yoann for coordination with the site consumer.
The page set can grow while preparing the opening; all six gallery pages
must exist before the release candidate. Do not publish incomplete placeholders.

## Body contract

English, short, one implicit H1 from `title`. The gallery is a learning path,
not a feature catalogue: each chapter opens with a practical question, builds
on the previous one, and leaves the reader with a concrete next experiment.
Every example is independently runnable; reading earlier chapters is optional.
Use the same H2 sections for every gallery page: `Run`, `Read the graph`,
`Expected result`, `Break it`, `Read the golden difference`, `Carry forward`,
`Limits`. `Read the graph` names the few relevant
parts of the referenced source; it does not duplicate that source.
`Carry forward` explains the transferable idea and links the adjacent published
chapters. Announce unfinished chapters without a broken link or placeholder.
Getting Started links the complete published example sequence. Fence examples
with their language (`sh`, `rkg`, `text`, `mermaid`). No imports, JSX, MDX or
embedded components.

### User Guide chapters

Each `guide/` chapter is 500 to 900 words and opens with “After this chapter
you can”. It explains one part of the language using exact slices of the
current files in `examples/`; removing common indentation is allowed,
rewriting a slice is not. Cite the file and line range in the surrounding
prose. The command-line chapter instead explains all commands and options;
its command syntax is checked against the binary's help. Do not introduce
another runnable example just for the guide.

End with `Summary` and `Next` sections. `Next` links the following chapter
and the hands-on example that demonstrates the same idea; the last chapter
continues into the examples. Link limitations to the Overview's `What is
available today` section. Cite normative specification paths as plain text
until the site renders the specification. Use no em dash, product
comparison, unmeasured claim or present-tense promise of an absent feature.
At most one Mermaid diagram belongs in a chapter, and only when it explains
something more clearly than the text. Internal links must resolve in the
pinned checkout, including heading fragments.

### Diagrams

Use Mermaid only, inline in the Markdown that explains it. Show one idea per
diagram, with about fifteen nodes at most. Name boxes and participants after
the real Nodes, Contracts and commands. Derive diagrams from checked source
or a versioned trace, never from imagined state. Do not use `%%{init}%%`
directives or make color or style carry meaning; the site applies its theme.
Put one lead sentence before each diagram saying what it shows.

`getting-started.md` is the source of the README quickstart. The Markdown between
`<!-- quickstart:start -->` and `<!-- quickstart:end -->` is copied verbatim
into the root README and checked by a repository test. No installation-time
claim is made before two unaided external trials have been measured.

Assets live in `assets/`. Record the scenario, viewport (1352 × 760), selected
transition and capture command in the opening ledger. A 30-second GIF and
three flagship captures are required, not generated diagrams of imagined state.
Editor syntax is `editors/vscode/rastok-dsl/syntaxes/rkg.tmLanguage.json`.

The gallery uses ordinary relative Markdown image links for these assets;
no new front-matter field is needed. `assets/webhook-capture.json` is capture
provenance, not a documentation page: the site must treat `assets/` as files,
not front-matter-bearing content. The replay GIF maps every frame to a recorded
transition and contains no generated scene or external credential.
