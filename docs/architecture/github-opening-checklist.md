# GitHub contribution opening checklist

18 September 2026. Local preparation following the relayed review by
Anthropic Claude Opus 5 (reasoning effort: high). This document does not
authorize a GitHub push, repository creation, settings changes, mail/DNS
provisioning or public release. The SharePoint bare backup is a separate,
explicitly authorized destination.

## Files prepared here

- `ci.yml` includes a read-only `DCO sign-off` job. It checks real PR head
  commits, not the synthetic merge commit, or the commits added by a push.
  A newly created branch checks its whole history. The author must have a
  matching parsed `Signed-off-by` trailer; another person's signature or a
  line in the message body is insufficient. The checker has local Git-based
  regression tests and uses only Node built-ins and Git.
- Bug, design and documentation issue forms request reproducible evidence
  and scope. Blank issues are disabled in the chooser; the PR template asks
  for human review, tests, contract references, and model/version credits.
  These are submission aids, not enforcement of every possible client.
- `SECURITY.md` and `CODE_OF_CONDUCT.md` name Yoann Pantic as recipient and
  `contact@rastok.com` as the planned private address. They explicitly mark
  activation and receipt testing as pending.

## Owner actions before opening — not performed

| Item | State | Evidence required |
|---|---|---|
| Contact mailbox or forwarding to Yoann | Pending | Provision `contact@rastok.com`; verify receipt of synthetic security and conduct reports from an external address; do not put the forwarding destination in Git |
| Private vulnerability reporting | Pending | Enable it for `rastok-io/rastok`; verify the reporting entry and maintainer notifications with a synthetic report |
| Labels | Pending | Create or confirm `bug`, `design`, `documentation`, `needs-triage`; verify the forms assign them. Missing labels are not created by an issue form |
| DCO and CI on the hosted repository | Pending | Run a signed sample contribution and an unsigned one, verify pass/refusal; record the real checks produced by the complete workflow |
| PR merge strategy | Decided by Yoann on 18 September; activation pending | Keep strict DCO for every commit. Enable only Rebase and merge, disable merge commits and squash, and require linear history. Verify author-matching sign-offs and Assisted-by trailers on the resulting main commits |
| Historical DCO exception | Maintainer decision pending | Commit `bb1b1edb3606` (6 August 2026) has no author sign-off; decide how to address it before the first hosted push, without silently exempting or rewriting it |
| Protection of `main` | Pending | Require PR review and the actual DCO/full CI checks; disable force pushes and deletion; explicitly review any bypass permissions |
| Contribution forms | Pending | Check each form and PR template in the hosted UI, including private-report links and no sensitive data in sample reports |

All of these settings belong to the repository/domain owner; local YAML and
Markdown do not activate them. Once the channels work, update the policies
with the verified contact/reporting instructions and record the evidence here.
Do not remove the pending notice merely because a hostname is registered.

The first push checks all introduced history. If legacy commits lack DCO
sign-offs, surface that finding for a maintainer decision; do not silently
skip them or rewrite history to make a check green.

Fable's review identified a separate integration risk: a new merge commit
without its author's sign-off fails the main push check even when all PR
commits passed. Yoann selected strict DCO and rebase-only integration, not a
`--no-merges` exemption. Rebase adds the reviewed commits without a merge
commit; squash is not a substitute for preserving their attribution footers.
The selected policy still needs hosted configuration and a test PR. No
GitHub setting or existing commit was changed here. See GitHub's
[merge strategies](https://docs.github.com/en/pull-requests/reference/pull-request-merges).

## Local verification — 18 September

All ten Git-fixture regressions pass: author matching, missing or wrong
sign-offs, parsed footers, unsigned earlier and merged-parent commits,
first-push history and invalid revision arguments, plus an unsigned merge
whose two branches are correctly signed. Workflow and issue-form
YAML parse locally. The hosted form rendering and Actions run remain untested.

The read-only history audit through `a1c1728` checks **328 commits** and
finds exactly one missing sign-off, `bb1b1edb3606` above. The checker therefore
correctly refuses that complete history. This is not a green historical DCO
claim or a reason to alter an old human certification automatically. Newly
created commits remain individually signed; the explicitly authorized
SharePoint backup is not a public contribution opening.

Reference syntax and platform behavior:
[issue forms](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms),
[template chooser](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/configuring-issue-templates-for-your-repository),
[workflow events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows).
