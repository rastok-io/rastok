# Public-opening showcase — 10 September 2026 brief

Opening: **5 November 2026**, freeze **24 October**, per the 5 October
[human-review amendment](human-review-m1.md). This is the current M1 delivery
checklist. **Release TODO:** relabel the first public package as format 1 before
publication. No public operation is authorized by this update. The
[current plan package](../spec/identity.md) and diagnostic recording **5**
are tracked in the [time-carrier inventory](time-carriers-identity.md). Yoann
approved witness regeneration on 24 October, preserving epoch-aligned tumbling.
The preceding movements remain documented in the
[lifecycle identity inventory](lifecycle-registry-identity.md) and the
[Connector identity inventory](connector-rework-identity.md).
The opening-tree freeze date follows C's independent replay and Yoann's
scheduling decision; the earlier freeze above is not a new scheduling decision.
Exact-instance
closure, resident Process, native Retry, flat annotation types and bounded
typed HTTP deserialization keep their execution semantics. The
[C4 ledger](retry-c4-qualification.md) records the preceding native-Retry
qualification; its 18 September counts and unchanged-witness claims below
are dated evidence, not the status of the subsequent compact representation.

The standard measurement is exactly `@measure(id, unit)`. Signed original
integer/decimal samples remain in the journal, including Retry-held attempts.
No report, automatic pricing, aggregate, subset policy, OTLP implementation or
Budget manager is required to close M1. The two future layers have explicit
boundaries in [observability](../spec/observability.md#72-planned-optional-otlp-export)
and the [Budget draft](budget-governance-draft.md).

The [release inventory](release-qualification.md) distinguishes the blocking
debugger browser pass from three **open, non-blocking** measurements: HTTP/3
server ngtcp2, Autobahn, and marginal Connector implementation cost. This is a
priority decision, not a passing qualification. Public push, tag and crate
publication require Yoann's order on or after 5 November. M2 durability is not
an M1 gate. The separate site consumes this repository's ordinary Markdown,
examples, goldens and editor grammar at one tag; it does not author them here.

**Publication day, on Yoann's order.** Three items found on 24 and 25 October
that no earlier checklist named: the workspace `Cargo.toml` still carries
`publish = false` and must drop it before `cargo publish` (rastok-core, then
rastok); the README's crates.io badge is written as an HTML comment and is
enabled once the crate exists; the `rastok` and `rastok-core` names were free
on crates.io on 24 October, and whether to reserve them before the opening is
Yoann's decision.

## Accepted opening corrections

Yoann approved the bounded-join destination correction before implementation.
An Operator's `within` or `last` bound makes `JoinExpired` productible even
without an authored emission. Both independent analyses must derive this;
removing its disposition is `E-UNWIRED-OUTPUT`, not a dead-output warning.
No automatic discard, changed runtime fact, plan bump or golden re-blessing.

Focused regression evidence: `v16_destinations` covers time/count bounds,
real destinations and explicit `nothing`, a nested Graph, a forged plan with
the expiry edge removed, and observable evictions. The three refusal probes
failed before the correction and pass after it. The unbounded case remains
dead when no handler emits on that port.

Yoann decided on 11 October that the plan and the runtime are realigned on
the specification's Node ontology before the opening
([D-2026-10-11-1](../spec/decisions.md); ledger section
[Realignment on the Node ontology](human-review-m1.md#realignment-on-the-node-ontology--decided-2026-10-11)):
one Node table with a body by nature, handlers inside their Operator, one
typed index, one reference wiring per Graph with derived execution indexes,
and execution separated from read-only inspection and debugger
presentation. No new semantics and no identity movement; the package
representation moves once more before publication and is what the format-1
relabel names. Lot 1's layout and embedded cost are now qualified in the
[closure report](node-realignment-qualification.md). The
[lot 2 closure](node-runtime-qualification.md) qualifies inspection separation
and the shared hosted Worker: 1,598 Rust tests, 35 browser tests, unchanged
artifacts and embedded footprints. Independent review and the external
opening gates remain separate; the calendar stays with Yoann.

## Delivery ledger

| Item | State | Evidence / next gate |
|---|---|---|
| Connector T5/T9 rework | Done — implemented and locally qualified; independent review next | Shared standard definitions, Driver interfaces/modules, per-Node named implementation selection and ownership recovery; read-only human/JSON catalogue and File contributor guide. The approved `std/process@6` / package-22 movement is [inventoried](connector-rework-identity.md). [Qualification](connector-rework-qualification.md) records 1,623 Rust tests, 35 browser tests, three no_std targets and both QEMU workloads, all passing. Public-opening gates remain separate. |
| Bounded-join expiry productibility | Done | 14 destination tests; 9 September combined baseline: 1,297 Rust tests passed |
| `run --record`, `replay [--ui]` | Done within the diagnostic contract | CLI and real-Chrome tests prove read-only prefix navigation and no Driver on replay; `@personal` and boot-dependent profiles still refuse persistence, as documented |
| Readable golden divergence | Done | Expected/actual labels, first record and field, retained causal context; shared CLI/UI path; classified values hidden; byte/seal comparison unchanged |
| Flat annotation types and generic business measurements | Done — executable two-field contract | `type Name = @struct { ... }` uses ordinary data identity and independently validated metadata. `@measure(id, unit)` retains signed integer/decimal original samples; monetary units have no special policy. Former cost/registry tags and arguments refuse by name. Stdlib/checker/core, typed deserialization, held-attempt retention and hostile-package tests are part of [C4](retry-c4-qualification.md); reports, subsets and pricing are removed, not unfinished. |
| `rastok new` | Done | One executable typed graph plus Lab; tests simulate it and prove no overwrite and path/name refusals |
| Optional JSON member presence | Done | Approved `has(object.member)`, core evaluator, independent package validation, constant folding, classification and pinned CEL differential tests; absence is distinct from null and observed zero; no witness re-blessing |
| Loop / Retry / Saga separation | Done for the admitted M1 profile | Finite outbound Call Retry retains the prepared operation, all original attempts and exact closure-based deadlines; final-only publication and exact-prefix recovery are qualified. Loop progresses state, Retry repeats an operation; same-owner composition and unsupported hosts refuse. Saga remains deferred. [C4](retry-c4-qualification.md) records the common blessing and the unchanged embedded limits. |
| `docs/site/README.md` schema | Done | Plain Markdown/YAML, exact requested fields; Rust validation rejects missing/unknown/duplicate fields and checks artifact references; no extra field introduced |
| Editor grammar | Done for the executable syntax, unchanged by format 20 | Data-only package, four lexical and three real TextMate/Oniguruma tests; all shipped examples, `@struct`, custom annotations, Retry, idempotence, JSON quoting and instance commands. No new dependency or claim of interactive historical-VS-Code certification. |
| echo page and comparison | Done | Three scenarios and gallery goldens; reproducible run/replay; `cli` versus `lab` provenance prevents a false claim of raw run/simulate seal equality |
| file-pipeline | Done | File graph with four scenarios/goldens, including `files.utf8`: three bytes, not two characters. CLI regression checks actual replacement, changes both files, and proves replay leaves them untouched |
| asymmetric two-tenants | Done | Bob once, Alice timeout/backoff/retry of identical prepared bytes; idle lifetime retains generations 0 and 1. No artificial attempt field. Both migrated goldens and batch/step equivalence pass; the genuine Loop UI fixture remains a separate regression. |
| HttpServer ack source simulation | Done — qualification below | Decoded requests use the ordinary inbound binding and correlated ACK without a physical Driver or artificial App input; source observations come from INGRESS |
| webhook-to-partner | Done locally; independent visual gate remains separate | Six native Retry scenarios and their versioned goldens pass; invalid-business-payload trace stays unchanged. Typed Receipt retains charges on original Held and Published emissions, without an extractor or report. ACK, deduplication, personal-data admission and redaction remain qualified. Fresh captures/GIF and break-it golden checks are delivered. |
| sensor-alarm | Done | New graph/page, three scenarios/goldens: per-sensor hysteresis, missing heartbeat, virtual day; warm CLI measurement published with its limits |
| agent-gateway | Done for resident and limited HTTP examples; no live HTTP-provider certification | `examples/agent-http.rkg` targets Responses with explicit projected JSON. Six first-time goldens, local physical HTTP and LIVE/HISTORY/REPLAY tests qualify original samples, missing versus zero, schema refusal, job/session META and secret-free retention. Inference is not declared idempotent. The resident bridge keeps its existing real-Process evidence; no paid provider call or report is claimed. |
| Rastok Codex Gateway browser continuation | Partial — client/native tests and user live smoke check; full visual pass open | `agent-chat.rkg` serves readable HTML/vanilla JS and isolates resident Processes by nickname. Three Lab scenarios, four Rust tests (including simultaneous HTTP/WebSocket listeners and exact Quit), ten client-script tests. Yoann reports three real conversations working. No runtime/format change or golden blessing; no automated paid model test. [Evidence](agent-gateway-showcase.md#readable-browser-continuation--17-september) distinguishes that smoke check from full browser qualification |
| Explicit conversation / child closure | Implemented — core/CLI qualification complete | `instance.stop` / `child.instance.stop`, exact generation, bounded queue, repeat-safe Closing, journal/recovery and recording v2. Package v18 is explicit; nonparticipating bodies/hashes/traces remain unchanged. Two-budget Session handoff follow-up: 1,356 tests passed, zero failed/ignored; three no_std targets and embedded heap rechecked. Missing/ambiguous targets refuse; an external input cannot guess a nested command's scope and refuses at check. This dated closure evidence is complemented by physical supervision in the resident-service ledger and local gateway browser tests. See [evidence](instance-stop-command.md#9-two-budget-session-shutdown--10-september) |
| Gallery learning path | Done for six published chapters | Book-style index, graph-reading guides, transferable ideas and previous/next navigation. Chapter 6 now references the resident gateway and links the primer; its usage/credential limits remain prominent. No front-matter schema expansion |
| Quickstart and README | Partial | One source block, exact-copy test, working `new` path and flagship/profile command; [two unaided trials](quickstart-trials.md) are prepared but not performed |
| Debugger CI | Partial — local suite green; first hosted run failed during browser setup | 35 isolated browser tests, 16 layout tests, 4 lexical editor tests and 10 chat-client tests pass locally. Three additional real-tokenizer tests pass locally using the site's installed engines. Rust projection tests are in the all-feature matrix. The first [hosted CI run, 40366152302](https://github.com/rastok-io/rastok/actions/runs/40366152302), on 26 October at `80f9729`, could not install Chromium: Playwright 1.58.2 does not support Ubuntu 26.04. The debugger runner is now pinned to `ubuntu-24.04`; the remote debugger job has not yet passed. |
| Browser opening gate | Partial | The six reported implementation/UX points now have focused regressions, including real intermediate advance states. An independent visual retest is still required; local automation does not certify it |
| Three flagship captures and 30-second GIF | Done locally — native Retry assets; publication review remains separate | `docs/site/assets/` records the current Retry graph at transitions 0/20/34, 1352×760, 120 frames / 30.000 s. The backoff view shows the real held attempt and due time, with no Loop dial. All three stills were visually inspected; a regression keeps page captions aligned with the capture manifest. No causal golden is changed |
| Three release binaries | Partial — local macOS evidence; hosted release build not dispatched | Both macOS targets rebuild with hosted-ui/hosted-websocket, without HTTP/3; starter and all three echo scenarios pass. Intel runs under Rosetta, not native hardware. The first [hosted CI run, 40366152302](https://github.com/rastok-io/rastok/actions/runs/40366152302) exists, but is not a release-artifact build. `alpha-artifacts.yml` has not been dispatched; its Linux build is now pinned to `ubuntu-24.04`. Linux release artifacts remain unqualified; the macOS release configuration is unchanged. |
| CHANGELOG, crate metadata and release tag | Partial | `0.1.0-alpha` manifests, exact core dependency, license/readmes and CHANGELOG prepared. Both crate archives build outside the workspace using the packaged local core. Confirmed public URLs are now in local metadata/readmes; prior archives predate this URL-only update. Publication stays disabled; authorized tag/publication remain pending |
| Public documentation destination | Partial — URL confirmed, availability unverified | `https://docs.rastok.com` must resolve before the authorized 5 November publication: minimal Starlight quickstart/gallery preferred, 301 to the actual `www.rastok.com` gallery as fallback. [Site handoff](../site/README.md#public-destination-handoff--18-september-2026); no deployment or DNS change performed |
| Contribution safeguards and private contact | Partial — files prepared, activation pending | Local DCO check and ten regressions, issue forms and PR template are present. Yoann selected strict DCO and rebase-only PR integration; hosted activation remains pending. `contact@rastok.com` is planned for Yoann, not yet activated or receipt-tested. Labels, main protection, private vulnerability reporting and one historical unsigned commit remain in the [owner checklist](github-opening-checklist.md). No GitHub, mailbox or DNS change performed |
| Paid benchmark wave | Partial — dry harness requalified | All 20 references pass their referees; 4 runner tests and 3 canned attempts pass (17 deliberately unattempted). Prompt v4 uses executable idempotence/Retry, preserving frozen v1–v3. Exact three model IDs/adapters and spending authorization remain user choices; no paid run started. |
| Optional concepts pages | Removed from this pass | No spare scope assumed; four short pages may follow the gallery |
State vocabulary: Done = fait; Partial = partiel (including an explicitly
unstarted or externally blocked item); Removed = retiré from this pass, not
silently claimed complete. Six chapters are published in the checkout; the
remaining independent visual, distribution and opening work stays explicit above.
Every result section below is dated; the live table and the human-review
identity ledger take precedence for the current format.

## Current representation qualification — 7 October

Package 20 and recording 4 are blessed once against the committed inventory.
The final ordinary workspace run passes **1,571 tests, zero failures/ignored,
61 groups**. Fmt, strict Clippy, rustdoc, three `no_std` targets and both fixed
Cortex-M33 workloads pass. The 28 gallery scenarios match their regenerated
goldens, including `files.utf8`; the three package witnesses match their
inventoried bytes. The [identity ledger](human-review-v20-identity.md) records
the initial native-startup failure and complete passing rerun separately.

No release archives or captures were regenerated, and no independent browser
review was performed in this format pass. The external opening rows remain
open; this qualification is not an authorization to publish.

## Dated local qualification — 18 September

**1,545 Rust tests pass, zero failures/ignored, 60 result groups** in the final
ordinary all-feature Cargo run. A separate six-worker campaign runs all 58
Cargo-built test executables with the same pass count. Two earlier local
listener-start failures and their unchanged passing reruns are retained in the
[C4 ledger](retry-c4-qualification.md), alongside the common fixture blessing.
Formatting, strict Clippy, rustdoc, three no_std checks, **35 browser tests**,
**30 other JavaScript tests** and **3 real-tokenizer tests** pass.

The eight gallery programs execute **27 versioned scenarios** and eight
intentional break-it edits. Echo/JSONL/gauge causal witnesses stay unchanged;
only the inventoried Retry traces and package/recording identities move.
Both macOS artifacts and extracted crate builds pass their local probes.
Fresh native Retry screenshots and the 30-second GIF are inspected; UML,
whitepaper, current specs, public pages and editor syntax describe v19.

M1 is **locally implemented and qualified, not fully signed off for opening**.
Remaining blocking evidence: independent browser review, two unaided
quickstart trials, Linux/remote artifact production, availability of the
confirmed documentation destination and the ordered release. The paid
benchmark is rehearsed, not authorized/executed.
The [contribution opening checklist](github-opening-checklist.md) also requires
working private reporting, hosted safeguards and a maintainer decision on the
single unsigned historical commit before public participation opens.
The in-app Browser is unavailable here; the existing Chrome harness is not
passed off as an external reviewer. No public push/tag/publication or paid
model call is performed.

### Prereview corrections — 18 September

Fable's prereview showed that the earlier claim of fully aligned public prose
was premature: the whitepaper and example catalogue still named v18 or an
unblessed candidate, and an instance example retained the removed `effect`
attribute. Those three discrepancies are corrected. Regression tests check
the current package/Contract names, reject retired syntax in public DSL
excerpts, and compile the whitepaper's Retry example against its real Contract.
The superseded roadmap checkpoints now read as history, and an orphan checker
comment is removed. Compiling CI commands use `--locked`, checked by a test.

Yoann chose strict DCO with rebase-only integration; hosted activation remains
pending in the [opening checklist](github-opening-checklist.md). The possible
quadratic Retry journal-scan cost is recorded as an M2 qualification debt in
the [release inventory](release-qualification.md#retry-journal-scan-debt-before-m2).
The suggested consolidation of two terminal-case lookups remains a separate
core cleanup, not a behavior fix delivered here. No runtime logic, format,
golden or repository history changes in this pass.

Focused verification: **10 site-content tests and 10 DCO regressions pass**.
Formatting, strict workspace/all-targets/all-features Clippy and rustdoc also
pass on this tree, with locked offline dependency resolution where applicable.
The full 1,545-test result above is the preceding qualification, not a fresh
workspace run for these documentation, CI and regression-test changes.

### Current-document cross-check — 21 September

The prereview corrections were still present in the local tree. A further
source/spec comparison removed remaining current-v17 labels, the whitepaper's
plan-phase `effect` attribute, and `E-LOOP-EFFECT` from Loop's active diagnostic
table. Native Retry is no longer described as an unimplemented target in the
delivery contract; its manager advances retained attempt facts, not authored
Loop commands. The optional partner key is distinct from an idempotence
guarantee and from future Saga requirements. Identity tables now consistently
describe recurrence timer references and retained Call closure time, without
changing any encoding. The diagnostic registry also reflects the corrected
bounded-join rule, two-field measurements and Contract/Driver separation.

Regression evidence: three documentation checks failed on the stale text,
then **12 site-content tests, 14 destination tests and 10 DCO tests passed**.
The whitepaper Retry fragment is compiled against the real Contract catalogue.
Formatting, strict workspace/all-targets/all-features Clippy and rustdoc pass. No full
workspace test run, new runtime qualification, golden change or publication
is claimed. Historical reports retain their original versions and counts;
F-06 and hosted activation remain separate from this documentary correction.

## Historical candidate qualification — 17 September

After projected Responses deserialization, browser corrections and explicit-unit
registry retirement, the final workspace/all-features/no-fail-fast run has
**1,527 passes, 9 failures, 0 ignored tests, 58 result groups**. The failures
are the inventoried candidate identity/recording/golden expectations, not
silently updated fixtures. The preceding cold-start run and the chat's passing
isolated/full reruns are preserved in the
[annotation migration evidence](annotation-types-m1.md#implementation-and-focused-qualification).
This is not a green-workspace or M1-closure claim.

Formatting, strict workspace Clippy, rustdoc and three `no_std` targets pass.
The rebuilt CLI passes **35 browser tests**, and **29 JavaScript tests** pass
without a new front dependency. Original measurements remain inspectable
without reports; no paid provider call or external reviewer was invoked.
The fixed embedded Retry/recovery workload has **2,020 B** of code margin and
**512 B** of heap margin. Final stdlib/A4 work, the single candidate blessing,
independent browser/quickstart review and current release artifacts remain open.

## Local close-out qualification — 14 September

Three implementation changes are independently committed: pure member
presence (`d0931ee`), real intermediate time-advance/debugger states
(`672ac5b`), and alpha crate/distribution preparation (`29c88f6`). The HTTP
candidate is retained in `docs/examples/`, not promoted into `examples/` or
the executable gallery. At this checkpoint its effect declaration was
undecided; D-2026-09-14-4 subsequently chose to remove the global requirement
through the refactor tracked above. That cutover is not part of these tests.

Executed locally:

- Full all-feature workspace: **1,393 passed, 0 failed, 0 ignored**, 47 suite
  results. The focused presence regression was first observed refusing the
  unsupported macro; the advance regression first observed six transactions
  hidden in one step. Both are now green. No causal golden was re-blessed.
- Strict Clippy, formatting, rustdoc and all three no_std compile targets
  pass. QEMU keeps 67 records and **111,732 B** heap; text plus rodata is
  **952,028 B**, leaving **96,548 B** under the unchanged code-size gate.
- Release-binary browser suite: **33 passed**, no console errors or skipped
  cases; layout **15**, editor **5**. Both pointer and keyboard inspect
  mailbox heads and original App outputs; a misleading DATA `tenant` field
  cannot replace the retained instance key. A line-wrap regression was
  observed failing before reserving space for ordinary META names.
- Flagship captures/GIF regenerated at 1352×760, transitions **0/23/39**,
  with **120 frames / 30.000 seconds**. This is real retained-state rendering
  and local visual inspection, not a new independent audit.
- Both crate archives verify after extraction using the packaged core as a
  local dependency until registry publication. Both macOS binaries create
  and simulate the starter; x86-64 is exercised under Rosetta. The first
  smoke invocation raced file creation in the shell harness; after awaiting
  creation, both sequential runs pass. Linux/remote CI are not run.
- All twenty benchmark references satisfy their referees; the three canned
  tasks pass and seventeen are explicitly unattempted by that canned author.
  The four runner tests pass. This is not the paid three-model wave.

Evidence directory: `/tmp/rastok-m1-close.buJbj0`, including
`workspace.log`, `browser-release.log`, `layout-editor.log`, `tokenizer.log`,
`clippy-final.log`, `doc.log`, `nostd-*.log`, `spike.log`, `spike-size.log`,
the package verification logs, `bench-referees.log` and `bench-dry/`.
The two local macOS archives and checked `SHA256SUMS` are under `dist/` there.
These are build artifacts, not public releases. A complete rerun against
the prepared alpha manifests is recorded separately in `workspace-final.log`.

Still required to close M1: the approved refactor and HTTP adapter's
execution evidence, Linux build, independent browser retest, two unaided
quickstart trials, the exact paid-wave identities/authorization, confirmed
public repository URLs and the explicitly ordered release. None is inferred
from passing local tests. No Fable/Claude invocation, paid model call, public
push, tag or registry publication was performed by this pass.

## Gateway and documentation follow-up — 14 September, before member presence

Scope explicitly limited by Yoann to the gateway and documentation. The
resident bridge now composes WebSocketServer with Process(service), preserving
native JSON-RPC frames and exact conversation ownership. The CLI qualification
uses two local sockets and a harmless shell child; it never invokes Codex.
The core keeps the installed Session source scope; the physical Driver holds
an immediate first frame behind its opening event. Both regressions were
observed failing before their respective local corrections. New gateway
goldens are added, not replacements for existing evidence. A third regression
aligns interactive scripted-world rounds with the batch runner: both gateway
scenarios now retain the same complete canonical journal when stepped. The
existing HTTP-source batch/step test also passes.

At this earlier checkpoint the HTTP one-model adapter was not closed because
safe inspection of an optional JSON member was missing. Yoann subsequently
authorized minimal `has(...)` under D-2026-09-14-3; the current delivery table
records its implementation and the subsequently approved effect-separation
refactor, whose implementation remains open. Model costs remain unknown rather
than fabricated from transport events. No permissive map-to-struct cast was
introduced.

Documentation assets now have their reproducible capture command in
[release-qualification.md](release-qualification.md#publication-assets--14-september).
The three stills were visually inspected and FFmpeg/ffprobe verifies the GIF's
1352×760 dimensions, 120 frames and 30.000-second duration. The tokenizer
qualification uses the site's installed TextMate 10.0.2 and Oniguruma engine
4.4.3, not a new repository dependency. No front-matter field was added; the
capture manifest under `assets/` is a file, not a page. The independent browser
audit, publication approval, timed external quickstart, binaries, release and
paid bench are neither executed nor declared closed in this pass.

Local final browser qualification: **31 passed**, zero failed/skipped; layout:
**15 passed**; editor: **5 passed** (three lexical and two real-tokenizer tests).
The initial new browser check named a nonexistent panel selector; it was
corrected to the actual `panel-instances`, then the complete browser suite
reran green. The runtime was not changed for that test-harness error. Evidence:
`/tmp/rastok-m1-browser-qualified.log`, `/tmp/rastok-m1-layout-final.log` and
`/tmp/rastok-m1-editor-final.log`.

The three no_std targets pass. Cortex-M33 echo still reports `SPIKE_OK`,
67 records, **111,732 B** heap and **951,468 B** text plus rodata, identical to
the resident-service checkpoint. This does not measure resident resources on
bare metal. The whitepaper now distinguishes the delivered resident bridge
from its future authenticated/usage adapter and no longer reports the fixed
`JoinExpired` defect as current. Historical audit measurements stay dated.

Current UML views 02, 03, 06 and 07 now name v18, resident Process Sessions,
exact-instance closure and diagnostic replay. The seven-diagram renderer
passes with the already installed PlantUML 1.2026.8; the four changed SVGs
were visually inspected. Views 01, 04 and archival 05 remain unchanged.
No diagram is offered as an executable API or a durability implementation.

Final Rust qualification: **1,374 passed, 0 failed, 0 ignored, 45 suite
results** with `cargo test --workspace --all-features --no-fail-fast`.
The gateway, unchanged echo/JSONL/gauge witnesses, all 21 gallery scenarios,
seven break-it edits, site contract and assets are included. Strict workspace
Clippy, fmt, `cargo doc --workspace --no-deps` and `git diff --check` pass.
Full transcript: `/tmp/rastok-m1-workspace-final.log`; ancillary evidence:
`/tmp/rastok-m1-clippy-qualified.log`, `/tmp/rastok-m1-doc-final.log`,
`/tmp/rastok-m1-nostd-final.log` and `/tmp/rastok-m1-spike-final.log`.
One cold-host integration attempt exceeded the initial ten-second CLI-startup
allowance before the executable bound its socket; the test now allows thirty
seconds for host loading, independently of handshake/frame timeouts, and the
full rerun passes. No golden was changed to mask either that harness issue or
the corrected runtime/Lab invariants. No commit, push, tag, external audit,
distribution-binary build, paid benchmark or model invocation was performed.

## Resident dialogue checkpoint — 14 September

The six-chapter learning path now includes the resident Process foundation;
the complete agent gateway remains partial in the delivery row above. The
public CLI runs two new dialogue scenarios and their initial goldens. The
gallery regression runs nineteen scenarios across six chapters and checks
each chapter's break-it edit. No earlier golden was re-blessed for service.

The [service qualification](process-service.md) records 1,368 green tests in
the complete all-feature run, followed by 378 library and 9 service tests for
the final host-teardown fence. Strict Clippy, fmt, docs, three no_std targets
and the embedded spike pass. This does not close the independent browser,
publication-assets or complete gateway-adapter gates.

## Prior CLI/gallery-pass evidence

No language semantics, plan format or runtime rules changed in this CLI and
gallery pass. Existing echo, JSONL and gauge goldens were not re-blessed.
New gallery traces are initial versioned witnesses;
the keyed example changes because the requested story is now asymmetric and
has an idle lifetime, not because an engine identity rule changed.

`gallery.rs` executes eleven scenarios across four pages through the real CLI
and compares each against its own trace. A second regression applies all four
published “break it” edits to temporary copies and requires a named record
divergence, expected/actual values and retained causal context, without blessing.
The broader executable-example catalogue passes twenty scenarios in ten programs.
The two-retry browser fixture pins the
original transition-specific regression cases; the public asymmetric example
also runs through the browser's LIVE/HISTORY/REPLAY matrix. `site_content.rs`
checks the front-matter interface, artifact existence and README-copy rule.
The source does not depend on Node to run those content checks.

Malformed golden hexadecimal (including non-ASCII input) refuses without a
parsing panic. Echo, JSONL and gauge were also rechecked through `rastok check`:
their hashes remain `5ffba3ea…`, `2f89c8b5…` and `0d3ad43d…` respectively.

CLI/gallery checkpoint qualification: **1,307 Rust tests passed, 0 failed,
0 ignored, 40 suite results** with `cargo test --workspace --all-features
--no-fail-fast`. The first run exposed two UI projection tests still coupled
to the old two-retry example; those tests now use the preserved two-retry
fixture, keeping their assertions intact. The complete rerun is green.
Fmt, all-target/all-feature Clippy, `cargo doc --workspace --no-deps` and
`git diff --check` pass. Full Rust transcript:
`/tmp/rastok-opening-tests.ulO7MD/workspace-qualified.log`.
The bare-metal and cross-target checks were not repeated for this CLI/content
pass; their last measured evidence remains the dated checkpoint below.

Local Chrome: **27 passed**, including record/replay, fresh-session shortcuts,
failed assertions, mailbox hovers and the two new File/sensor examples.
Layout: **13 passed**. Editor lexical probes: **3 passed**, explicitly not a
full TextMate tokenization test. Browser captures are in the temporary evidence
directory `/tmp/rastok-opening-tests.ulO7MD/screens`, not falsely presented as
the missing flagship release assets. [Tooling provenance](opening-tooling.md)
records the external test runner and the build-only release workflow.

Sensor timing uses an external monotonic observer around three warm **debug**
CLI launches of all three scenarios: **0.012118 s, 0.008899 s, 0.007616 s**.
Compilation and golden comparison are excluded. A cold observation was 10.94 s;
these are observations on the reference host, not universal throughput claims.

## HttpServer Lab continuation

Yoann selected simulation through the executable Contract without its physical
Driver (D-2026-09-09-3). This supersedes the proposed artificial App harness:
the production graph is unchanged. Existing source-injection syntax now binds
the HttpServer ack model. Respond and Session source scripting remain named
gates; no raw HTTP injection or `expect refused` syntax is added under v17.

`lab_http_source.rs` checks independent runs, batch/step equality, unchanged
production hash/package, nested source provenance, resolved body types,
source observation windows and check-time refusals. The source Driver tests
exercise acceptance-after-commit, rejection without a terminal, and retention
under mailbox pressure. The UI names the real source in its pending inspector
and reads admitted roots from the selected journal prefix, not a fake delivery.
The corresponding Chrome regression uses `fixtures/lab-http-source.rkg`.

Current continuation qualification: **1,316 Rust tests passed, 0 failed,
0 ignored, 41 suite results** (`cargo test --workspace --all-features
--no-fail-fast`); all six `lab_http_source` tests also pass with
`--no-default-features`. Fmt, all-target/all-feature Clippy and documentation
builds pass. Echo, JSONL and gauge retain the three hashes recorded above;
no existing golden or v17 package identity was changed by this continuation.
The first matrix exposed an incorrect expectation in the new refusal test:
`listener.in` is rejected by direction, not as an unknown port. The test now
requires that more precise diagnostic; the final matrix is green.

Chrome: **28 passed**, including the source preview and admitted INBOUND
inspection; layout: **13 passed**. The final browser run used an isolated
`RASTOK_BIN` after an initial run collided with a featureless build replacing
`target/debug/rastok`. A separate redaction regression failed before the fix
and passes for both pending and admitted source copies: a plan containing
classified data does not expose the raw, possibly earlier unclassified source
DATA through these new views. This changes presentation, not journal contents
or annotation propagation. Evidence: `/tmp/rastok-http-lab.1ocABm/`, including
`workspace-qualified.log`, `featureless-qualified.log`,
`browser-isolated-qualified.log` and `source-redaction-before.log`.
Bare-metal and cross-target gates were not rerun for this hosted Lab change.

This scripts decoded requests and successful fixed ACKs. The existing real
`auth_codec_and_body_refusals_never_invent_an_inbound_call` test remains the
wire/auth/body qualification; a green graph simulation is not that evidence.
This ingress checkpoint preceded the flagship continuation below. The complete
six-page gallery remains open until agent-gateway is delivered.

## Webhook flagship continuation

`examples/webhook-to-partner.rkg` starts at the real HttpServer Contract, with
no artificial App input. The Lab scripts decoded bodies and the fixed 202 ACK;
the partner Call uses the same typed framing, secret reference and journal as
real execution. The synthetic deployment file is mandatory. Its environment
coordinate contains no credential; the page supplies an explicitly fictional
fixture value and does not grant production privacy authority.

The six scenarios cover nominal delivery, timeout then success, bounded
exhaustion, an ignored duplicate and idle-retention expiry, decoded shape and
business rejection after ACK, and a 500 response deliberately retried by the
graph. The partner's required external contract is idempotent upsert by event
id with a stable idempotency key; the changing attempt is audit-only. The
`seen` state implements acceptance deduplication for five idle minutes,
including after a permanently failed job, without restart durability.

`webhook_showcase.rs` compares independent full journals, counts actual
OUTBOUND Calls, verifies that missing privacy refuses before any record,
searches canonical records for a credential canary, inspects every UI
transition for email/credential disclosure and folds exact reported costs.
The request adapter explicitly erases the field schema into JSON `variant`,
retaining security taint on the whole body. The accounting adapter explicitly
narrows the charge scalar to decimal(38,18); it does not assume that a struct
type-test recursively rescales nested decimals. The 500 trajectory reports
0.03 USD at that flow point; timeouts without reported charges have no row,
not a zero bill.

Two regressions failed before correction. Policy derivation now carries the
left side of `&&` into the right side's type/case context, preserving both
operands' taint, as the existing expression contract requires. The debugger
withholds dynamic mailbox/delivery DATA in classified plans, closing the
raw-copy path before a typed validator. Neither correction changes v17,
Connector Contracts, the runtime journal, or the three existing witnesses.
Only the new flagship's initial source/golden artifacts are established here.

The local integrated Browser skill could not initialise after its plugin
update because its service still referenced a removed plugin version. No
manual visual pass is claimed. A passing snapshot regression is not the
independent opening browser gate; captures and the GIF remain unfinished.

## Suspension checkpoint — 9 September 2026

Work was suspended at the user's request. All changes remain in the worktree;
no commit, push, tag or publication was performed. The verification processes
and the local flagship debugger started for this continuation were stopped.

- The six flagship scenarios pass through the public CLI with their new
  candidate goldens. Existing echo, JSONL and gauge witnesses are unchanged.
- The focused suite last completed with three passes and one failure: its
  cost expectation incorrectly expected separate 0.01 and 0.02 USD rows where
  the public report correctly aggregates 0.03 USD. The expectation is fixed,
  but the final suite was interrupted before its result; do not claim four
  passing tests yet.
- The short-circuit policy regression passes. Fmt and all-target/all-feature
  Clippy pass. `cargo doc` and the featureless flagship test run were
  interrupted, not qualified.
- The all-feature workspace run was interrupted during the CLI suite. Its
  library suite had 373 passes and two HTTP/3 failures:
  `http3_uses_verified_tls_and_preserves_the_response_stream` and
  `http3_uses_the_same_retained_call_and_graph_response`. These failures need
  investigation; the workspace is not reported green.
- Core's Cortex-M3 check passes. The Cortex-M33 spike reports `SPIKE_OK`,
  111,452 B peak heap and 929,896 B code, within the existing gates.
- The integrated browser could not initialise; no manual visual pass or new
  release captures are claimed.

Local evidence is in `/tmp/rastok-webhook.gBp8ox/` (temporary, not a durable
artifact). On resumption: rerun the focused flagship and gallery suites,
investigate the two HTTP/3 failures, complete workspace/featureless/doc gates,
then qualify the browser and release assets. Preserve the v17 freeze and do
not re-bless existing goldens to hide a failure.

## Resumed qualification — 10 September 2026

The suspension above is historical. The corrected cost assertion now passes:
the full flagship suite has four passing tests. Its six CLI scenarios and the
five gallery pages' seventeen scenarios match their goldens; every published
"break it" edit produces the expected record-level divergence without blessing.

- `cargo test --workspace --all-features --no-fail-fast`: **1,321 passed,
  0 failed, 0 ignored**, across 42 suite results including doc-tests.
- Fmt, all-target/all-feature Clippy, `cargo doc --workspace --no-deps` and
  `git diff --check` pass.
- `cargo test -p rastok --target aarch64-apple-darwin --no-default-features
  --test webhook_showcase --test lab_http_source`: **9 passed**. The explicit
  native target keeps this build separate from the concurrently qualified
  all-feature binary. No physical HTTP Driver is required to simulate the
  source or the partner.
- Core checks pass for `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi` and
  `riscv32imac-unknown-none-elf`. The last QEMU heap/code measurement remains
  the dated 9 September checkpoint; it was not rerun for these hosted tests.
- Chrome: **29 passed**; layout: **13 passed**; editor lexical probes:
  **3 passed**. The added browser test exercises the flagship's six scenarios
  in LIVE, HISTORY and REPLAY, checks the first admission and snapshots for
  email/bearer disclosure, and checks exact nominal/HTTP-500 costs.
- Echo `5ffba3ea…`, JSONL `2f89c8b5…` and gauge `0d3ad43d…` remain unchanged.
  No tracked golden, v17 format, semantic Contract or dependency was changed.

The two HTTP/3 failures were reproduced before correction. Their fixtures
assumed IPv4 while `localhost` resolved to IPv6 first. Matching the listener
to that resolution fixes both tests without changing production networking
or disabling DNS-name certificate verification. They pass in the focused
rerun and the full workspace; the independent ngtcp2 gate remains open.

The full Rust run also encountered slow executable startup on this host. A
sample of a waiting CLI child showed `_dyld_start` before application entry;
the final completed run, not the apparent stall or an interrupted run, is the
evidence counted above. Transcripts are under `/tmp/rastok-resume.JSfPSN/`:
`workspace.log`, `featureless-native-target.log`, `browser.log`,
`layout-editor.log` and the individual gate logs. These are temporary local
evidence, not release artifacts.

Actual flagship captures revealed a readability defect (corrected in the
follow-up below): fixed
layout geometry is squeezed by initial fit while screen-sized text, instance
badges and port labels collide. The next visual pass must test their rendered
bounds on this denser graph, not just the existing small-layout fixtures.
The captures remain diagnostic only. The integrated browser connection and
independent audit, approved captures/GIF, agent-gateway, timed quickstart and
release preparation remain open; this checkpoint does not close M1. No commit,
push, release tag, paid model run or publication was performed.

### 10 September — flagship graph readability

The front-only correction retains the same snapshots, v17 plan and journal.
A new Chrome regression first failed on 23 overlapping text pairs at initial
admission. Compound scope bands now reuse horizontal rank columns; node
dimensions reserve title/instance space before admission. Automatic fit has a
readability floor shared by font compensation and label obstacles; on narrow
viewports the drawing remains pannable. The full flagship fits at 1352×760.
Direct cubic curves remain direct when clear; deterministic detours avoid
node bodies, scope headers and port labels. HISTORY no longer covers zoom.
Mailbox badges grow for four-digit occupancy, without changing routes.

The final embedded-asset binary, not the local asset-override development
harness, passes **30 Chrome tests**. The geometry regression covers transitions
0, 1, 8 and completed plus narrow width, rendered title size and containment,
text intersections and curve/label clearance. **15 layout tests** and **3
editor lexical tests** pass. Desktop and narrow captures were viewed locally
under `/tmp/rastok-graph.IgAGKl/screens/`; logs are in the parent directory.
They are diagnostic evidence, not the three approved release captures or GIF.

The completed workspace rerun passes **1,321 Rust tests**, **0 failed**, **0
ignored**, across 42 suite results. Fmt, all-target/all-feature Clippy,
`cargo doc --workspace --no-deps` and `git diff --check` pass. Echo, JSONL and
gauge retain their pinned hashes. The initial Rust pass caught a source-string
test pinning the previous pixel gutters; that smoke test now checks the shared
readability-floor wiring, with actual clearance covered by layout/browser
regressions. `workspace-final.log` is the completed green rerun; the earlier
`workspace.log` retains that failure. Newly linked executables again spent
time before application entry on this host; listing test cases in parallel
only warmed those executables and is not counted as executed tests.

The integrated browser bootstrap still fails on its missing older service
module; no independent audit was requested. Agent-gateway, timed quickstart,
release preparation and the other named opening gates remain open. No
dependency, format, semantic Contract or golden was changed in this pass.

## Persistence and personal data

The durable deployment journal remains M2 work. A diagnostic recording is
nevertheless a persistent copy and cannot inherit the in-memory journal's
run-scoped privacy capability. The first recording reader/writer refuses
plans carrying personal data before creating a file or starting any effect.
Secrets remain late-bound Driver material and must never enter the recording.

For M2, effective storage limitation and targeted erasure/externalization are
capabilities to qualify, not a declaration that a retention field alone makes
storage compliant. A retention-only design must still explain how applicable
erasure requests are honoured before ordinary expiry and how lawful retention
exceptions are handled. UI masking is not erasure. Rewriting causal plaintext
cannot preserve its old seal and byte-identical replay; erasure must expose
that consequence, or use a separately reviewed externalization design.
The capability must cover all retained copies: DATA, META, identifying
instance keys, indexes, exports and backups. Erasing one displayed payload
does not establish erasure of the journal's other identifying material.
See the [CNIL retention guidance](https://www.cnil.fr/fr/passer-laction/les-durees-de-conservation-des-donnees)
and [right-to-erasure guidance](https://www.cnil.fr/fr/comprendre-mes-droits/le-droit-leffacement-supprimer-vos-donnees-en-ligne).

No new durable capability or GDPR compliance certificate is claimed in M1.
The six pages retain the brief's common contract: one command, expected
result, an induced failure, explained golden difference and visible limits.

## 9 September combined qualification (prior checkpoint)

- CLI: a real File read is recorded; removing the data file and `.rkg` source
  does not change replay output or causal seal. Existing captures are never
  overwritten. Personal-data persistence refuses before file creation.
- Browser: 25 real-Chrome headless tests pass, including recording navigation,
  effective replay pause and absence of execution POSTs; 13 layout tests pass.
  This is automated evidence, not closure of the independent visual audit.
- Rust: 1,297 all-feature tests pass across 38 suite results, no failure or
  ignored test. Fmt, all-target/all-feature Clippy and `cargo doc` pass. The
  two recording CLI probes and Clippy also pass after the final writer-API
  simplification: finishing writes the file; reading returns its exact
  checkpoint history rather than an in-memory single-checkpoint substitute.
- Portability: core checks on Cortex-M3 and RV32IMAC pass. Cortex-M33 QEMU
  reports `SPIKE_OK`, 111,452 B peak heap and 928,576 B code; gates unchanged.
- Identity: echo `5ffba3ea…`, JSONL `2f89c8b5…`, gauge `0d3ad43d…`; no golden,
  package version, dependency or Connector Contract changed. The three
  expense-approval scenarios, including expiry, pass through the public CLI.

The dated checkpoint above is not the current delivery ledger. The table at
the top owns the status of each opening item; unfinished rows are not waived
by test counts. The public-opening date remains 5 November 2026.

## 10 September — learning path and Process contract review

The gallery now reads as a book: a practical question opens each chapter,
`Read the graph` identifies its new mechanism, and `Carry forward` connects
the transferable idea to the adjacent available chapters. The index offers
both the complete reading order and the short echo-to-flagship route. The
public front-matter schema, graph paths and golden paths are unchanged.

The editorial source is the sibling strategy repository's `NARRATIF.md`
(10 September), with `STRATEGY.md` and the gallery section of `DEMONSTRATION.md`:
build before partners are available; share execution rules; retain difficult
cases as regression scenarios; then apply the same model to agents. Older
strategy implementation counts and the one-job-per-message gateway description
are not copied as current evidence. The six-chapter path still has only five
available pages; no placeholder service example is presented as executable.

Yoann changed the gateway target to one resident Process per conversation
and requested a complete Process definition before a reduced implementation.
The [contract review](process-contract-review.md) records job/service lifecycle,
protocol ownership, framing and capacity, timeouts, Closing/restart, security,
recovery, the Lab and an acceptance matrix. Its repository audit identifies
the sheet/profile and Session-correlation gaps. The RPC and resource-policy
choices are not sealed; the checker still refuses `service` by name. No
production Rust, Contract id, plan version or golden changed in this pass.

Local qualification: **1,322 Rust tests pass, 0 failed, 0 ignored, 42 suite
results** with workspace/all-features/no-fail-fast. This includes the gallery's
golden and induced-difference proofs and five site/editor contract tests. The
new content regression first failed on missing graph-reading/continuation
sections; it now enforces chapter order and previous/next links. Fmt,
all-target/all-feature Clippy, `cargo doc` and whitespace checks pass.
No browser/site render or no_std/QEMU qualification was rerun for this
documentation/test-only change; those gates retain their earlier status.

### Subsequent protocol agreement

D-2026-09-10-2 settles the responsibility previously left open above: Process
parses complete JSONL stdout frames into `variant`; Operators validate and
interpret the conversation protocol. Several turns may use one resident child,
and `exit` remains the child's terminal rather than a turn-completion signal.
The unused Process-owned `correlate`/RPC and protocol-resume promises have been
superseded in stdlib and the composition sketch. Other Contracts' correlators
and the executable job mode are unchanged.

The subsequent D-2026-09-10-3 fixes fresh opening: child death produces its exit
without automatic respawn; a new input may open a new child in the same
still-open instance. No previous write is resent, no child memory is presumed
restored, and a stopped generation is not revived. The review inventory adds
the required death / no-input / new-input / stale-event qualification scenario.

Service execution remains deferred. Exact caps, resource selection across
WebSocket/Process Sessions, termination policy and Lab world actions still
require decisions in the review inventory. These documentation-only agreements
allocate no version and change no runtime, package or golden.
