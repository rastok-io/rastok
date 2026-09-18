# Rastok v19 syntax

Data-only VS Code extension: no activation code, dependencies, language server
or build. The site's highlighter loads `syntaxes/rkg.tmLanguage.json` at the
same tag as the public documentation. Language id: `rkg`; scope: `source.rkg`.

For local development, launch VS Code with
`--extensionDevelopmentPath=<absolute-path-to-this-directory>` and open an
`.rkg` file. Marketplace publication is not part of this slice.

This original Apache-2.0 grammar follows the current repository lexer and the
[VS Code TextMate contribution format](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide).
No seed assets or third-party grammar were copied. Highlights cover declarations,
ports, Extensions, manager commands, case routes, matches, annotations, Lab
actions, per-instance closure, comments, literals and the three built-in readers.
The v19 update adds Retry settings, `idempotent`, projected JSON and `@struct`;
authored annotation names use the same application scope as standard ones.
Highlighting an annotation does not claim a supported runtime interpretation.
The removed global `effect` attribute is no longer an executable keyword.
JSON readers respect strings and nested objects/arrays; raw text and base64 do
not accidentally highlight their contents as DSL. It does not validate types
or routes; `rastok check` does. Future guest compilers must supply their own
region grammar; no SQL/jq execution or semantic tokens are advertised here.
The declared VS Code minimum is a compatibility target, not a claim of a
measured historical-editor test.

## Qualification

The dependency-free lexical smoke tests also run in repository CI:

```sh
node --test editors/vscode/rastok-dsl/grammar.test.cjs
```

The full tokenizer test uses installed TextMate and Oniguruma modules through
explicit paths. It checks nested readers/comments, command scopes, and every
shipped example returning to the host grammar. Missing tools fail explicitly.
No npm install, build step or Node dependency is added to the compiler.

```sh
RASTOK_TEXTMATE=/absolute/path/to/vscode-textmate/dist/index.js \
RASTOK_ONIGURUMA=/absolute/path/to/engine-oniguruma/dist \
node --test editors/vscode/rastok-dsl/tokenizer.test.mjs
```

Qualified on 14 September with the site's existing
`@shikijs/vscode-textmate` 10.0.2 and `@shikijs/engine-oniguruma` 4.4.3
(MIT-licensed tooling), using its WASM Oniguruma engine: five tests green
including lexical smoke tests. This is tokenizer evidence, not an independent
interactive VS Code usability audit. The site can run the same test at its
pinned content tag, without copying this grammar or modifying this repository.

Requalified on 18 September against v19 with the same installed engines:
**four lexical + three real-tokenizer tests pass**. The added regressions cover
`@struct` as a declaration rather than an annotation application, authored
annotation names, `Retry` settings and the ordinary `idempotent` attribute.
No new front-end dependency was introduced.
