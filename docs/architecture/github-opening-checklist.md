# GitHub contribution opening checklist

18 September 2026. Local preparation following the relayed review by
Anthropic Claude Opus 5 (reasoning effort: high). This document does not
authorize a GitHub push, repository creation, settings changes, mail/DNS
provisioning or public release. The SharePoint bare backup is a separate,
explicitly authorized destination.

## Files prepared here

- `ci.yml` includes a read-only `DCO sign-off` job. It checks real PR head
  commits, not the synthetic merge commit, or the commits added by a push.
  A newly created branch checks its whole history after the one documented
  exception below (`bb1b1edb3606`). The author must have a
  matching parsed `Signed-off-by` trailer; another person's signature or a
  line in the message body is insufficient. The checker has local Git-based
  regression tests and uses only Node built-ins and Git.
- Bug, design and documentation issue forms request reproducible evidence
  and scope. Blank issues are disabled in the chooser; the PR template asks
  for human review, tests, contract references, and model/version credits.
  These are submission aids, not enforcement of every possible client.
- `SECURITY.md` and `CODE_OF_CONDUCT.md` name Yoann Pantic as recipient:
  `security@rastok.com` for vulnerabilities, `contact@rastok.com` for conduct
  reports, as confirmed on 27 October. Activation and receipt testing remain
  unverified; changing a document does not provision either channel.

## Owner actions before opening

| Item | State | Evidence required |
|---|---|---|
| Reporting mailboxes or forwarding to Yoann | Configured on 3 November 2026 (Exchange Online; MX, SPF, DKIM and DMARC records checked); external receipt test at T-20 | Confirm activation and external receipt for `security@rastok.com` (security) and `contact@rastok.com` (conduct); do not put the forwarding destination in Git |
| Private vulnerability reporting | Pending | Enable it for `rastok-io/rastok`; verify the reporting entry and maintainer notifications with a synthetic report |
| Labels | `design` and `needs-triage` created on 3 November 2026; `bug` and `documentation` already existed; form assignment still to verify in the hosted UI | Create or confirm `bug`, `design`, `documentation`, `needs-triage`; verify the forms assign them. Missing labels are not created by an issue form |
| DCO and CI on the hosted repository | Partial — hosted rerun green; test PRs pending | On 26 October 2026, the private repository's first [CI run #1, ID 40366152302](https://github.com/rastok-io/rastok/actions/runs/40366152302) ran on `main` at `80f9729`. The history contains 452 commits; hosted DCO passed for the 378 commits after the configured exception `bb1b1edb3606`. A separate local audit of all 452 finds only that documented exception. The macOS gate (1,649 tests, summed from the raw job log), all three no_std targets, spike-emulated and bench-proofs passed. The Ubuntu gate exhausted disk while compiling `retry_runtime`; debugger setup failed because Playwright 1.58.2 does not support Chromium on Ubuntu 26.04. Ubuntu 24.04, Linux toolset/image cleanup and `line-tables-only` dev/test debug information are now validated by [40375545442](https://github.com/rastok-io/rastok/actions/runs/40375545442), attempt 2: all jobs passed (Ubuntu about eight minutes, macOS three, debugger four). Attempt 1 exposed the intermittent macOS HttpServer refusal-close race and cancelled Ubuntu; the [T9 follow-up](human-review-m1.md#hosted-ci-and-alpha-follow-up--26-october-evening-yoann--astra) records the bounded close fix, independent matrix legs and action-runtime upgrades awaiting their next hosted run. Signed and unsigned sample PR pass/refusal tests remain pending. |
| PR merge strategy | Decided by Yoann on 18 September; activation pending | Keep strict DCO for every commit. Enable only Rebase and merge, disable merge commits and squash, and require linear history. Verify author-matching sign-offs and Assisted-by trailers on the resulting main commits |
| Historical DCO exception | Decided 7 October (Yoann): documented exception, no rewrite | Commit `bb1b1edb3606` (6 August 2026) has no author sign-off and is kept as it is; `ci.yml` starts a first-push check after it, every later commit is signed, and `CONTRIBUTING.md` states the exception |
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

**7 October decision (Yoann).** The one unsigned commit stays unrewritten: a
rewrite would change 275 later commit ids, the private backup and the hashes
cited by the review ledgers. `ci.yml` now pins the first-push base to
`bb1b1edb3606` (exclusive) and `CONTRIBUTING.md` names the exception. Verified
locally: `check-dco.mjs bb1b1edb3606 <head>` passes every later commit, and
the all-zero base still refuses exactly that commit.

Reference syntax and platform behavior:
[issue forms](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/syntax-for-issue-forms),
[template chooser](https://docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests/configuring-issue-templates-for-your-repository),
[workflow events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows).
