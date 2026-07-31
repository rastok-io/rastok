# Specification errata

> Known internal contradictions in the normative corpus, awaiting a
> spec-side fix. Rule 1 of [README.md](README.md) applies: when two
> documents disagree, the designated home wins; the entries below name
> the losing text so it is corrected rather than silently reinterpreted.
> An erratum is not a license to improvise — until the fix lands, the
> designated home is the contract.

## E-2026-07-31-1 — grammar.md still targets the retired builder API

- **Where:** [grammar.md](grammar.md), section "What the parser must
  produce".
- **Contradiction:** the section requires a `.rkg` source to compile to
  "the same `Graph` value as the Rust builder API"
  (`Graph::new` / `add` / `from_config`), a relic of a retired
  prototype. The designated homes disagree:
  [identity.md §1](identity.md) defines the checked form as the
  canonical HIR, and `WHITEPAPER.md` §2 names the compilation target
  the `ExecutablePlan`.
- **Resolution direction:** rewrite the section to state that the
  parser feeds the pipeline `AST → HIR → ExecutablePlan`; no Rust
  builder API is part of the contract.
- **Found by:** adversarial architecture review, 2026-07-31.

## E-2026-07-31-2 — `theorem` wording vs its grammar productions

- **Where:** [grammar.md](grammar.md), "`theorem` blocks" vs the formal
  productions (`theorem_decl`, `lab_body`).
- **Contradiction:** the prose says `theorem` "is not part of the
  grammar", while the formal grammar defines `theorem_decl` as a parsed
  reserved block. The intended behavior (parse structurally, refuse
  with an explicit `unsupported` diagnostic) is stated in
  [simulation-semantics.md §11](simulation-semantics.md).
- **Resolution direction:** editorial — reword the prose to "parsed as
  a reserved surface and refused with `unsupported`; carries no
  executable semantics in this corpus".
- **Found by:** adversarial architecture review, 2026-07-31.
