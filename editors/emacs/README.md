# Rastok for Emacs

`rastok-mode` detects `.rkg` files, colors Rastok sources and configures the
standard comment commands. It uses only Emacs libraries. It does not provide
completion, semantic navigation or an indentation engine. Run `rastok check`
for validation. Emacs 27.1 is the minimum compatibility target; the native
tests below measure the installed version, not every historical version.

## Install from a checkout

Use a checkout containing `editors/emacs`: the development branch, or the
0.1.1-alpha tag. A `rastok-examples` checkout can supply these
files whether the compiler was installed from an archive or with Cargo.

Add this to your Emacs configuration, replacing the absolute path:

```elisp
(add-to-list 'load-path "/absolute/path/to/rastok-examples/editors/emacs")
(require 'rastok-mode)
```

Reopen an `.rkg` file: the mode line shows `Rastok`. Colors follow the theme.
Use `M-;` (`comment-dwim`) or `M-x comment-region` for `//` comments.
No package archive or additional dependency is required. After updating the
checkout, restart Emacs or reload the mode before reopening the buffer.

## Test without loading personal configuration

From the repository root:

```sh
emacs -Q --batch -L editors/emacs \
  -l rastok-mode-test.el -f ert-run-tests-batch-and-exit
```

The tests use temporary buffers, not personal files. They cover file detection,
comment commands, Lab counts, declarations, types, manager commands, raw readers,
JSON strings, physical line boundaries, edits to reader delimiters and every
published example. Syntax processing recomputes reader boundaries from the
buffer start; no performance claim is made for generated or very large sources.
