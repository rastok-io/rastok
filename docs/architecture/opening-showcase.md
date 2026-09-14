# Public-opening showcase — 10 September 2026 brief

Opening: **15 October 2026**. The v17 opening freeze has one explicit v18
amendment: exact-instance closure under D-2026-09-10-1. All other semantics
remain frozen except for the subsequently requested resident Process Contract:
D-2026-09-14-1 admits `process@3` inside unchanged v18; its
[qualification ledger](process-service.md) supersedes the review's admission
gate. D-2026-09-14-3 separately authorizes minimal pure member-presence
inspection, without a package-version or journal move. This ledger records
delivery, not new semantic authority.
The subsequent D-2026-09-14-4 approves the
[Loop / Retry / Saga refactor](retry-extension-refactor.md); its implementation
and versioned cutover are still ahead. Current executable packages remain v18.
The [explicit instance-closure design](instance-stop-command.md) is now
executed through bounded retained requests, with an explicitly versioned
package and recording; its qualification ledger does not claim Process service.
The [release inventory](release-qualification.md) distinguishes the blocking
debugger browser pass from three **open, non-blocking** measurements: HTTP/3
server ngtcp2, Autobahn, and marginal Connector implementation cost. This is a
priority decision, not a passing qualification. Public push, tag and crate
publication require Yoann's order on or after 15 October. M2 durability is not
an M1 gate. The separate site consumes this repository's ordinary Markdown,
examples, goldens and editor grammar at one tag; it does not author them here.

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

## Delivery ledger

| Item | State | Evidence / next gate |
|---|---|---|
| Bounded-join expiry productibility | Done | 14 destination tests; 9 September combined baseline: 1,297 Rust tests passed |
| `run --record`, `replay [--ui]` | Done within the diagnostic contract | CLI and real-Chrome tests prove read-only prefix navigation and no Driver on replay; `@personal` and boot-dependent profiles still refuse persistence, as documented |
| Readable golden divergence | Done | Expected/actual labels, first record and field, retained causal context; shared CLI/UI path; classified values hidden; byte/seal comparison unchanged |
| `rastok new` | Done | One executable typed graph plus Lab; tests simulate it and prove no overwrite and path/name refusals |
| Optional JSON member presence | Done | Approved `has(object.member)`, core evaluator, independent package validation, constant folding, classification and pinned CEL differential tests; absence is distinct from null and observed zero; no witness re-blessing |
| Loop / Retry / Saga separation | Partial — contract approved, implementation not started | D-2026-09-14-4 distinguishes state iteration from protected attempts over one recurrence engine and removes the global effect gate at cutover. [C0–C4](retry-extension-refactor.md#8-implementation-order-and-exits) track algebra, identity, implementation and qualification; no format change is claimed yet |
| `docs/site/README.md` schema | Done | Plain Markdown/YAML, exact requested fields; Rust validation rejects missing/unknown/duplicate fields and checks artifact references; no extra field introduced |
| Editor grammar | Done for the executable v18 syntax | Data-only package, three lexical tests and two real TextMate/Oniguruma tests; all shipped examples, JSON quoting, raw readers, nested comments and instance commands. Future guest compilers and semantic tokens remain outside scope; no interactive historical-VS-Code certification |
| echo page and comparison | Done | Three scenarios and gallery goldens; reproducible run/replay; `cli` versus `lab` provenance prevents a false claim of raw run/simulate seal equality |
| file-pipeline | Done | New File graph, three scenarios/goldens and page; CLI regression checks actual replacement, changes both files, and proves replay leaves them untouched |
| asymmetric two-tenants | Done | Bob once, Alice timeout/backoff/retry; idle lifetime scenario asserts generations 0 and 1; two-simultaneous-retry UI regression retained as a separate fixture |
| HttpServer ack source simulation | Done — qualification below | Decoded requests use the ordinary inbound binding and correlated ACK without a physical Driver or artificial App input; source observations come from INBOUND |
| webhook-to-partner | Done — executable graph, audit profile, page and regression qualification | Six scenarios, including explicit HTTP 500 handling; deterministic journals, bounded Calls, attestation refusal, exact costs and snapshot/browser redaction pass. Visual assets are delivered below; malformed-wire refusal is qualified by real HTTP conformance, not decoded Lab injections |
| sensor-alarm | Done | New graph/page, three scenarios/goldens: per-sensor hysteresis, missing heartbeat, virtual day; warm CLI measurement published with its limits |
| agent-gateway | Partial — resident WebSocket bridge delivered; HTTP usage/credential adapter awaits the approved refactor | Native-JSON protocol scenarios, exact conversation ownership and independent resident children are qualified. Minimal `has` is implemented. D-2026-09-14-4 resolves the design question: ordinary Calls need no Saga classification. The candidate still refuses under the current effect gate; implementing the cutover and qualifying the adapter remain required, with no invented idempotence. See [evidence](agent-gateway-showcase.md) |
| Explicit conversation / child closure | Implemented — core/CLI qualification complete | `instance.stop` / `child.instance.stop`, exact generation, bounded queue, repeat-safe Closing, journal/recovery and recording v2. Package v18 is explicit; nonparticipating bodies/hashes/traces remain unchanged. Two-budget Session handoff follow-up: 1,356 tests passed, zero failed/ignored; three no_std targets and embedded heap rechecked. Missing/ambiguous targets refuse; an external input cannot guess a nested command's scope and refuses at check. This dated closure evidence is complemented by physical supervision in the resident-service ledger and local gateway browser tests. See [evidence](instance-stop-command.md#9-two-budget-session-shutdown--10-september) |
| Gallery learning path | Done for six published chapters | Book-style index, graph-reading guides, transferable ideas and previous/next navigation. Chapter 6 now references the resident gateway and links the primer; its usage/credential limits remain prominent. No front-matter schema expansion |
| Quickstart and README | Partial | One source block, exact-copy test, working `new` path and flagship/profile command; [two unaided trials](quickstart-trials.md) are prepared but not performed |
| Debugger CI | Partial | Current browser/layout/lexical suites wired into CI; locally 33 browser, 15 layout and 5 editor tests pass on 14 September. The two optional full-tokenizer tests reuse the site's installed highlighter, outside CI. Remote workflow not run |
| Browser opening gate | Partial | The six reported implementation/UX points now have focused regressions, including real intermediate advance states. An independent visual retest is still required; local automation does not certify it |
| Three flagship captures and 30-second GIF | Done — regenerated after fine-grained advance and locally inspected | `docs/site/assets/`: timeout/retry scenario at transitions 0, 23 and 39, 1352×760; GIF has 120 real retained-state frames and verified duration 30.000 s. Reproduction script and manifest supplied; publication approval and independent browser audit remain separate |
| Three release binaries | Partial | macOS arm64/x86-64 release builds with `hosted-ui,hosted-websocket` succeed; Intel echo is exercised under Rosetta, not native Intel hardware. Linux and remote workflow remain unrun; no HTTP/3 in these artifacts |
| CHANGELOG, crate metadata and release tag | Partial | `0.1.0-alpha` manifests, exact core dependency, license/readmes and CHANGELOG prepared. Both crate archives build outside the workspace using the packaged local core; publication stays disabled. Public repository URLs and the authorized tag/publication remain pending |
| Paid benchmark wave | Partial — dry harness requalified | All twenty reference graphs satisfy their referees; the three canned attempts and four runner tests pass. Exact three model IDs/adapters, provider spending cap and publication manifest still require selection and execution order; no paid run started |
| Optional concepts pages | Removed from this pass | No spare scope assumed; four short pages may follow the gallery |
State vocabulary: Done = fait; Partial = partiel (including an explicitly
unstarted or externally blocked item); Removed = retiré from this pass, not
silently claimed complete. Six chapters are published in the checkout; the
complete gateway demonstration remains partial as detailed above.

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
by test counts. The public-opening date remains 15 October 2026.

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
and the executable job profile are unchanged.

The subsequent D-2026-09-10-3 fixes fresh opening: child death produces its exit
without automatic respawn; a new input may open a new child in the same
still-open instance. No previous write is resent, no child memory is presumed
restored, and a stopped generation is not revived. The review inventory adds
the required death / no-input / new-input / stale-event qualification scenario.

Service execution remains deferred. Exact caps, resource selection across
WebSocket/Process Sessions, termination policy and Lab world actions still
require decisions in the review inventory. These documentation-only agreements
allocate no version and change no runtime, package or golden.
