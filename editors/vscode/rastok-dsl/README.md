# Rastok syntax

One TextMate grammar, packaged as a data-only VS Code extension and importable
as a JetBrains TextMate bundle: no activation code, dependencies, language server
or build. Language id: `rkg`; scope: `source.rkg`.
The documentation site loads `syntaxes/rkg.tmLanguage.json` from its selected
Rastok checkout. Development can use the working tree; a production build needs
its pinned vendor checkout refreshed to include grammar updates. The site's
old seed fallback is not the current language authority.

This original Apache-2.0 grammar follows the current repository lexer and the
[VS Code TextMate contribution format](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide).
No seed assets or third-party grammar were copied. Highlights cover declarations,
ports, Extensions, manager commands, case routes, matches, annotations, Lab
actions, per-instance closure, comments, literals and the three built-in readers.
The grammar covers Retry settings, `idempotent`, projected JSON and `@struct`;
authored annotation names use the same application scope as standard ones.
Highlighting an annotation does not claim a supported runtime interpretation.
The removed global `effect` attribute is no longer an executable keyword.
JSON readers respect strings and nested objects/arrays; raw text and base64 do
not accidentally highlight their contents as DSL. It does not validate types
or routes; `rastok check` does. Future guest compilers must supply their own
region grammar; no SQL/jq execution or semantic tokens are advertised here.
The declared VS Code minimum is a compatibility target, not a claim of a
measured historical-editor test.

## VS Code

From the repository root, open a development window with the extension loaded:

```sh
code --new-window \
  --extensionDevelopmentPath="$PWD/editors/vscode/rastok-dsl" \
  "$PWD/examples/agent-chat.rkg"
```

The `.rkg` association selects **Rastok** automatically. If necessary, use
**Change Language Mode** in the command palette and select **Rastok**.
On macOS, if `code` is unavailable, run **Shell Command: Install 'code' command
in PATH** from the command palette first. The development flag loads the local
extension in that window; it does not publish or install it globally.

For regular windows on macOS/Linux, copy this whole directory to
`~/.vscode/extensions/rastok-dsl/`, then restart VS Code. The destination must
contain `package.json`, not another nested `rastok-dsl` directory. Refresh that
copy after grammar changes; the development-window method reads the checkout
directly. No Marketplace publication or package download is needed.
See the official [development-host flags](https://code.visualstudio.com/api/working-with-extensions/testing-extension#advanced-setup-your-own-runner)
and [local extension loading](https://code.visualstudio.com/api/working-with-extensions/publishing-extension).

## RustRover / JetBrains

1. In **Settings → Plugins → Installed**, ensure **TextMate Bundles** is enabled.
2. Open **Settings → Editor → TextMate Bundles**, then click **+**.
3. Select the entire `editors/vscode/rastok-dsl` directory, which contains
   `package.json` — not just the JSON grammar file or the `syntaxes` directory.
4. Apply and reopen an `.rkg` file. If it remains plain text, check whether
   **Editor → File Types** explicitly associates `*.rkg` with another file type.

JetBrains accepts VS Code-format bundles through its
[TextMate integration](https://www.jetbrains.com/help/idea/textmate.html).
There is no separate native Rastok plugin. Colours come from the IDE's theme;
completion, type checking and navigation are not supplied by this grammar.
Use `rastok check` for semantic validation.

## Qualification

The dependency-free lexical smoke tests also run in repository CI:

```sh
node --test editors/vscode/rastok-dsl/grammar.test.cjs
```

The full tokenizer test uses installed TextMate and Oniguruma modules through
explicit paths. It checks reader nesting, non-nesting block comments,
line-bounded strings, type/call distinctions, command scopes, and every shipped
example returning to the host grammar. Missing tools fail explicitly.
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

Tokenizer qualification on 18 September used the same installed engines:
**four lexical + three real-tokenizer tests pass**. The added regressions cover
`@struct` as a declaration rather than an annotation application, authored
annotation names, `Retry` settings and the ordinary `idempotent` attribute.
No new front-end dependency was introduced.

Requalified on 29 September against the current parser and expression lexer:
**six lexical/bundle tests and six real-tokenizer tests pass** with the same
installed engines. The corrections cover the source types `long`, `double`,
`number`, `table`, `date`, `time`, `uuid`, `timestamp_ntz`; lifecycle `start`;
`now()` and constructor calls; unsigned `42u`; expression single quotes; and
ordinary strings ending at the physical line boundary. `decimal(18,6)` remains
a type while `decimal("1.25", 18, 2)` is a constructor. `list` and `tuple` are not
built-in source type spellings. Block comments end at the first `*/`, exactly
like `trivia::comment_at`; embedded `//` or `/*` never hides that closer.
No compiler, language rule or serialized artifact changes. This is tokenizer
qualification, not an interactive VS Code or RustRover installation audit.
