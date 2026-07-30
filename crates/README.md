# Rust crates

Rastok's compiler and runtime live in this Cargo workspace.

The initial `rastok` crate is deliberately empty: it gives the repository
a valid build root without pre-empting the implementation blueprint.
Additional crates should be introduced only when a real dependency or
compilation boundary justifies them.
