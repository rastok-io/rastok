# Contributing to Rastok

Rastok welcomes focused bug reports, scenarios, design criticism,
documentation, and code. Maintainer attention is finite, so the
contributor and reviewer share responsibility for keeping each change
understandable.

## Before writing code

- Search existing issues, specifications, and implementation first.
- For a bug, provide a minimal reproduction or failing scenario.
- For a non-trivial feature, semantic change, new dependency, or new
  crate boundary, open a design issue and wait for agreement before
  implementing it.
- Ask to take an existing issue when possible. A large unsolicited
  implementation may be closed even if substantial work went into it.

Small, self-contained fixes do not need ceremony.

## Pull-request contract

A pull request should:

- solve one stated problem;
- explain the user-visible result or invariant;
- identify the relevant issue or specification section;
- explain important alternatives and why they were rejected;
- include focused tests;
- update documentation when behavior changes;
- avoid unrelated refactors, formatting, generated files, and
  dependency churn;
- be small enough for a reviewer to understand without reconstructing
  the author's intent.

Passing CI is necessary, not sufficient. A change can compile and still
be wrong for Rastok's semantics or direction.

The repository-wide engineering rules are in
[`AGENTS.md`](AGENTS.md). They apply regardless of whether the code was
written manually or with tools.

## Rust checks

Before requesting review, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
```

Explain any check that cannot be run.

## Dependencies

Every new dependency must be justified in the pull-request description:

1. What capability is needed?
2. Why are the standard library and current dependencies insufficient?
3. What are its license, maintenance state, and significant transitive
   costs?
4. What would replacing it later require?

Convenience alone is not enough. Conversely, do not reimplement
security-sensitive or protocol-heavy code to satisfy a dependency-count
theory.

## AI-assisted contributions

AI assistance is allowed. Outsourcing review work to the project is
not.

Every submitted line of code must have been reviewed by a human before
submission, whether or not AI was involved. For AI-assisted or
AI-generated code, this means reading the complete diff and verifying
it against the repository, the relevant specification, and the stated
tests. Reviewing only a model summary or relying on green checks is not
human review.

By opening a pull request, the submitter attests that this human review
has taken place.

The person submitting a contribution is its author and is fully
responsible for:

- understanding and being able to explain every submitted line;
- correctness, security, tests, licensing, and provenance;
- checking that the work is relevant and does not duplicate existing
  code;
- removing fabricated claims, APIs, references, and unnecessary
  abstractions;
- responding to review in their own understanding rather than merely
  relaying prompts between the maintainer and a model.

Substantial tool assistance must be disclosed in the pull-request
description, for example:

```text
Assisted-by: <tool and model, if known>
Human verification: <what was reviewed and tested>
```

Commits may carry the same credit as a trailer, in the precise form
used by this repository (see [AGENTS.md](AGENTS.md)):

```text
Assisted-by: <company> <model> <version>
```

The `Signed-off-by` line is always the human contributor's own
certification — never an assistant's — and every commit carries one
(`git commit -s`).

Disclosure is contextual information, not a penalty. The same quality
bar applies to assisted and unassisted work.

Do not submit:

- raw model output;
- speculative rewrites or broad drive-by cleanup;
- a patch whose purpose the submitter cannot explain;
- generated tests that merely mirror the implementation;
- reports or security claims that the submitter has not reproduced;
- a second implementation of functionality already present;
- inflated prose intended to make a trivial change look substantial.

Maintainers may ask for a reproduction, a smaller patch, or a
human-written explanation before reviewing code. They may close a
contribution without line-by-line review when its likely value does not
justify its review cost. Repeatedly submitting fabricated or
unreviewed material may lead to contribution restrictions.

We do not try to detect which model wrote a patch. We evaluate whether
the human contributor has done the work required to make the patch
valuable and reviewable.

## Developer Certificate of Origin

Every commit must be signed off:

```sh
git commit -s
```

This adds a `Signed-off-by: Name <email>` line certifying the
[Developer Certificate of Origin](https://developercertificate.org):
that you wrote the change, or otherwise have the right to submit it
under the Apache License 2.0. The sign-off transfers no rights to the
project beyond that license — it attests provenance, and it is the
formal counterpart of the human-review attestation above. CI rejects
commits without a sign-off.

## Review outcomes

A maintainer may:

- merge the change;
- request focused revisions;
- ask for a design discussion first;
- close it as out of scope, duplicative, premature, or too costly to
  review.

Closing a contribution is not a judgment on the contributor. A clear
negative decision protects both the roadmap and volunteer time.

## License and conduct

By submitting a contribution, you license it under the repository's
Apache License 2.0 terms. Contributors retain their copyright; no CLA
or copyright assignment is required. See
[GOVERNANCE.md](GOVERNANCE.md) for the full contribution-rights model.

Participation is governed by
[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). Security reports follow
[`SECURITY.md`](SECURITY.md).
