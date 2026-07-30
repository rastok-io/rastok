# Rastok engineering instructions

This file applies to the entire repository. It is written for coding
agents, but it also records the engineering standard expected from human
contributors.

## Before changing anything

1. Read `README.md`, `ROADMAP.md`, and the relevant files under
   `docs/spec/`.
2. Search the repository before introducing a type, helper, dependency,
   diagnostic, or implementation. Do not recreate what already exists.
3. State the invariant or user-visible result the change protects.
4. Keep the patch scoped to that result. Unrelated cleanup belongs in a
   separate change.

The specification is the current semantic authority. Code must not
silently invent, weaken, or reinterpret a specified rule. If code and
specification disagree, expose the disagreement instead of choosing the
more convenient side.

## Engineering posture

- Prefer the smallest design that makes the invariant obvious.
- Add an abstraction only when it carries meaning, enforces a
  constraint, or supports a real second implementation.
- A little local duplication is cheaper than a premature shared
  abstraction. Consolidate once the shared concept is understood.
- Do not hide obvious code behind decorative helpers, wrappers,
  builders, traits, macros, or type aliases.
- A named constant must add domain meaning, units, policy, or a shared
  invariant. `CANONICAL_HASH_BYTES = 32` is useful;
  `EIGHT = 8` is not.
- Optimize from evidence. Do not contort ownership or APIs to avoid a
  cheap clone without a measured reason.
- Comments explain invariants, trade-offs, or non-obvious constraints.
  They do not narrate the syntax.

Elegance means that the design has fewer accidental states and fewer
surprises. It does not mean maximum abstraction.

## Rust style

- Use safe Rust. `unsafe` is forbidden in the current workspace.
  A future need for it requires an explicit design review, a narrowly
  isolated boundary, documented safety invariants, and dedicated tests.
- Prefer algebraic data types, exhaustive `match`, newtypes for
  genuinely distinct domain values, and constructors that reject
  invalid states.
- Keep the semantic core as pure as practical. Pass time, randomness,
  environment, I/O outcomes, and policy decisions through explicit
  inputs or effect boundaries.
- Functional style is a tool, not a contest. Iterator chains are welcome
  when they read linearly; a loop with local mutation is preferable when
  it is clearer.
- Use `Result` and typed errors for expected failure. Add context at
  system boundaries. Do not erase useful distinctions into strings too
  early.
- Avoid `unwrap`, `expect`, `panic!`, `todo!`, and `unimplemented!` in
  production paths. A test may use them when failure is unambiguous.
- Introduce a trait for a real contract or substitution seam, not
  automatically for every struct.
- Prefer concrete types until polymorphism is required. Prefer generics
  for closed, performance-sensitive substitution and trait objects only
  for genuine runtime heterogeneity.
- Keep async at I/O boundaries. Do not make a pure layer async merely
  because its caller is.
- Make ownership obvious. Borrow when the lifetime is local and clear;
  own data across durable, queued, or asynchronous boundaries.
- Keep modules cohesive and public surfaces small. `pub` is a design
  decision, not a default.

## Determinism and effects

Rastok's central promise makes hidden nondeterminism a correctness bug.

- No wall clock, random source, environment lookup, network access, or
  filesystem observation in the semantic core.
- Do not rely on hash-map iteration order or scheduler order in
  canonical output, identities, diagnostics, tests, or goldens.
- External outcomes re-enter the core as explicit, typed events.
- Preserve causal information. Do not turn a semantic distinction into
  a log-only detail.
- A golden change requires an explanation of the semantic reason. Never
  update a golden merely to make a test pass.

## Dependencies

- Start with the standard library, but do not reimplement cryptography,
  parsers, protocols, or storage primitives merely to avoid a sound
  dependency.
- Add a crate only when it removes more risk or complexity than it
  introduces.
- Before adding one, record the capability needed, why existing code is
  insufficient, maintenance status, license, transitive cost, and the
  exit strategy.
- Do not add a dependency for syntax sugar, one trivial helper, or an
  abstraction we do not yet need.
- Expose third-party types only when they are intentionally part of the
  public contract.

## Tests

- A semantic change requires a focused test.
- A bug fix starts with a regression that fails for the right reason.
- Test observable behavior and invariants, not private implementation
  choreography.
- Parser tests cover success, rejection, precedence, associativity, and
  diagnostic location.
- Canonicalization tests include boundary values and independently
  constructed equivalent values.
- Tests are deterministic, isolated, and independent of execution
  order. Real time and network access require an explicitly marked
  integration test.
- Keep fixtures small enough that a reviewer can understand why every
  field exists.

## Commits

- Write the subject in the imperative mood. The body explains why the
  change exists and what invariant or result it protects, not which
  files changed.
- Sign every commit (`git commit -s`). The sign-off is a human
  certification under the Developer Certificate of Origin: it carries
  the committer's own identity, matches the commit author, and is
  never made in an assistant's name.
- Credit substantial machine assistance with one trailer per
  assistant, placed before the sign-off:

  ```text
  Assisted-by: <company> <model> <version>
  ```

  Example: `Assisted-by: Anthropic Claude Fable 5`. No email address
  on this line. The pull-request disclosure rules in
  [CONTRIBUTING.md](CONTRIBUTING.md) apply unchanged.

## Agent-specific conduct

- Do not expand the requested scope because adjacent work looks useful.
- Do not replace working code wholesale when a local change is enough.
- Do not invent APIs, commands, files, test results, or external facts.
- Read generated code line by line before presenting it.
- Report uncertainty and unrun checks explicitly.
- Preserve user changes and avoid mass formatting unrelated files.
- Summarize why the change exists, not merely which files changed.

## Completion checks

Run the checks relevant to the change:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
```

If a check cannot run, say exactly why. Never describe an unrun check as
passing.
