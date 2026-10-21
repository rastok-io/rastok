# Rust crates

Rastok's compiler and runtime live in this Cargo workspace, as one
bootstrap crate.

`rastok` carries the whole M0 slice — lexer through checker and
planner, the Rastok expression chain, the journal and transactional Execution, the real
`Process` adapter, the lab runner, the canonical trace projection,
and the `rastok` binary (`check` / `run` / `simulate`). The seams the
architecture names (compiler, runtime, boundary, lab, golden) are
modules of this crate; additional crates should be introduced only
when a real dependency or compilation boundary justifies them.
