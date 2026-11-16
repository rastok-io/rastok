# Rastok for Vim and Neovim

The same runtime detects `.rkg` files, colors Rastok sources and sets buffer-local
comment options. It has no plugin-manager dependency or language server.
It does not provide completion, semantic navigation or automatic indentation.
Use `rastok check` for validation. Original `vi` without Vim syntax support is
not supported.

## Install from a checkout

Use a checkout containing `editors/vim`: the development branch, or the
0.1.1-alpha tag once published. A `rastok-examples` checkout can also supply
these files; installing the compiler from an archive or Cargo makes no
difference to the editor setup.

In your Vim configuration, or Neovim's `init.vim`, add the runtime directory
before enabling file types and syntax. Replace the absolute path below:

```vim
set runtimepath^=/absolute/path/to/rastok-examples/editors/vim
filetype plugin on
syntax enable
```

For a Lua-based Neovim configuration, the equivalent is:

```lua
vim.opt.runtimepath:prepend("/absolute/path/to/rastok-examples/editors/vim")
vim.cmd("filetype plugin on")
vim.cmd("syntax enable")
```

Reopen an `.rkg` file. `:set filetype?` and `:set syntax?` should both report
`rastok`. `:setlocal commentstring?` reports `// %s`. Colors follow the editor's
theme. Updating the checkout updates the runtime; reopen the buffer afterward.

## Test without loading personal configuration

From the repository root, with Node and each editor installed:

```sh
node --test editors/vim/test.mjs
RASTOK_VIM=nvim node --test editors/vim/test.mjs
```

The test opens an unsaved temporary buffer, loads no user configuration and
disables swap and history files. It checks detection, comment options, Lab
counts, declarations, types, manager commands, nested reader payloads,
non-nesting comments, line-bounded strings and every published example.
An absent editor fails the test rather than silently skipping it.
