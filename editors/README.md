# Editing Rastok sources

These integrations detect `.rkg` files and provide syntax highlighting.
They do not provide a language server, completion, semantic navigation or
type checking. Run `rastok check` to validate a program. The Lab debugger is
a read-only viewer, not a source editor.

| Editor | Integration and installation |
| --- | --- |
| VS Code, VSCodium | [Data-only VSIX](vscode/rastok-dsl/README.md#vs-code) |
| RustRover and other JetBrains IDEs | [TextMate bundle](vscode/rastok-dsl/README.md#rustrover--jetbrains) |
| Vim, Neovim | [Native syntax and file-type runtime](vim/README.md) |
| Emacs | [Native `rastok-mode`](emacs/README.md) |

The native modes can be loaded from the same checkout as the examples.
It works whether Rastok itself was installed from an archive or with Cargo.
Installation instructions are opt-in; no repository command edits personal
editor configuration.
