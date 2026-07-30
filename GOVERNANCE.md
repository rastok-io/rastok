# Rastok — Governance

**Status:** early-stage solo-maintainer governance. It will evolve as
the project grows and contributors join.

This document describes how decisions are made in the Rastok project,
who has the authority to make them, and how that authority may evolve
over time. It is intentionally short.

## Current state

Rastok is currently a **solo-maintainer project**. The maintainer is:

- Yoann Pantic

The maintainer has final authority on:

- the design of the conceptual model ([WHITEPAPER.md](WHITEPAPER.md));
- the DSL grammar ([docs/spec/grammar.md](docs/spec/grammar.md));
- the Rust runtime architecture;
- the open-source license ([LICENSE](LICENSE)) and its evolution
  (subject to the commitments in [OPEN_SOURCE_COMMITMENT.md](OPEN_SOURCE_COMMITMENT.md));
- the trademark policy ([TRADEMARKS.md](TRADEMARKS.md));
- the project roadmap.

This is a deliberate choice for the bootstrapping phase. Premature
committee governance would add coordination before the project has a
contributor base to govern.

## How decisions are made today

- **Code changes**: follow [CONTRIBUTING.md](CONTRIBUTING.md) → pull
  request → reviewed by maintainer → merged, refined, or closed.
- **Design changes**: open an issue tagged `design` → discussion →
  decision recorded in a follow-up commit or doc update.
- **Significant scope changes** (new major features, breaking changes,
  license discussions): a public discussion in the canonical
  repository's issue tracker, with at least one week of notice before
  the maintainer takes a decision.

## How contributions are received

Inbound contributions are accepted under the project's existing license
(Apache 2.0) by virtue of submitting them: *inbound = outbound*. There
is no Contributor License Agreement (CLA) and no copyright assignment.

Every commit must carry a Developer Certificate of Origin (DCO)
sign-off from its author. The DCO attests provenance; it grants the
project no rights beyond the Apache License 2.0. The mechanics are
described in [CONTRIBUTING.md](CONTRIBUTING.md).

Contributors retain copyright over their contributions.

The contribution quality bar, including the policy for AI-assisted
work, is defined in [CONTRIBUTING.md](CONTRIBUTING.md).

## How this governance will evolve

As the project grows, the governance is expected to evolve in stages:

### Stage 1 — Solo maintainer (current)

One maintainer. Decisions are fast. Suitable while the initial
implementation and contributor base are forming.

### Stage 2 — Maintainer + trusted reviewers

A small number of trusted reviewers can approve PRs without the
maintainer's individual review for changes within their declared
scope. The maintainer retains veto power and final authority.
Suitable when the contributor base reaches ~5–10 active people.

### Stage 3 — Technical Steering Committee (future)

A formal TSC with documented charter, voting rules, and rotation
mechanisms. Suitable when the project is widely adopted and
maintainers come from multiple organizations.

We are not in a hurry to reach Stage 3. The risk of premature
formalization is higher than the risk of staying nimble for too long.

## Project independence

Rastok is and will remain:

- governed through this repository by the project's maintainer(s);
- subject to the [Open Source Commitment](OPEN_SOURCE_COMMITMENT.md),
  which protects the project from being relicensed without a public RFC
  and a 12-month notice period.

Any organization that provides services around Rastok is governed
separately and does not acquire authority over the open-source project
by doing so.

## Communication channels

Until the project grows, all communication happens through the
canonical project repository:

- **Issue tracker** — bugs, feature requests, design discussions.
- **Merge or pull requests** — code and documentation changes.
- **RFC threads** — open questions and significant design proposals,
  initially hosted in the issue tracker.

A Discord, Slack, or mailing list may be added later if community size
warrants it.

## Code of Conduct

By participating in this project, you agree to abide by the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Contact

For governance-related questions, open an issue tagged `governance`.
For private or sensitive matters (security, conduct), contact
the maintainer through the public contact address in their profile on
the canonical repository.
