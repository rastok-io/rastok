# Public documentation content contract

Version: 10 September 2026. This directory belongs to the Rastok repository.
The separate `rastok-site` repository consumes it at one explicit Git tag,
together with `examples/`, the referenced goldens and the editor grammar.
There is no site runtime, Node dependency, MDX or component import here.

## Public destination handoff — 18 September 2026

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
references and duplicate gallery orders. Empty strings and empty lists do not
meet a required field. No field is optional in the following tables.

Every page has:

| Field | Type | Meaning |
|---|---|---|
| `title` | string | Page title; the site emits the single H1 |
| `description` | string | One-sentence summary for readers, search, JSON-LD and `llms.txt` |
| `order` | positive integer | Navigation order within the containing section |

Every page in `gallery/` additionally has:

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
The index links the complete published sequence. Fence examples with their
language (`sh`, `rkg`, `text`). No imports, JSX, MDX or embedded components.

`quickstart.md` is the source of the README quickstart. The Markdown between
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
