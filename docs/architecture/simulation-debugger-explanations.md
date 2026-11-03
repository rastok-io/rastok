# Simulation debugger — explanatory projection qualification

Date: 2026-09-08. Authority: the user's simulation-debugger brief of the same date.
Scope: `simulate --ui`, read-only explanation of a retained snapshot and journal
prefix. This is not another package-format or execution-controller change.

## Change separation and invariants

The preceding host-terminal format change is committed separately as `0e68b1f`.
The preceding UX work is committed as `dbd5078`. This pass follows both, with no
new dependency, package version, Contract version, journal record or execution
command. No golden is re-blessed.

Compiler source-arm locations are debug side metadata, outside the executable
plan. The Lab exposes the last assertion's existing observation window. Execution
inspection exposes its existing shared mailbox capacity. Neither accessor
changes admission, consumption, deadlines, state or commit. The timeline's action
index is a disposable view field, not a semantic fact.

The read model re-derives decisions with the exact consumed envelopes, named
join inputs, pre-transaction state, immutable plan bindings, retained time and
Loop context. No later record contributes to an earlier explanation. Closed
arms and ordered predicate results use the existing pure evaluator. Source
highlighting excludes unselected nested arms.

Predicate/assertion evidence is conservatively unavailable for classified
programs; opaque input and unavailable bindings remain explicit limits. The
redactor still precedes presentation. No explanation creates a new plaintext
channel or decodes an opaque payload merely to make the display look complete.

## Delivered surfaces and evidence

The per-item test map lives in [ui.md](../spec/ui.md#evidence-for-this-presentation-pass).

- Transition/action vocabulary, initial pause hint, action transition count and
  automatic next-action explanation; existing commands retain their meaning.
- Folded identifiers, copy/causal links, canonical instance-key labels and a
  fuel-aware Loop dial. The waiting indicator is asserted at B21 and B22.
- Focusable type, destination, Contract, Extension, capacity, occupancy and
  mailbox-head hovers. Mailbox heads show five redacted envelopes at most; the
  complete retained FIFO remains in its paged tab.
- Closed/open/nested Operator decisions and selected case-routed edges.
  `keyed-retry` actually has `model.exit -> decide`; the separate
  `ui-explanation.rkg` fixture exercises an edge `match`. No missing edge is
  invented to satisfy the brief's example spelling.
- Clickable envelope lineage and exact produced/consumed command-to-Admit links;
  Done commands do not acquire a fictitious next admission.
- Bidirectional authored-action/timeline selection, pending deliveries and
  retained deadlines, failure evidence including `expect port == nothing`, keyboard
  navigation/search, and two-column instance/session comparison.

All browser pages collect both uncaught exceptions and console errors. This
additional check exposed an existing inline SVG style rejected by CSP and a
missing favicon response. Sizing now uses the CSSOM; the authenticated favicon
request returns 204. CSP is not weakened. Browser teardown closes the SSE
consumer before terminating its private server, without filtering error messages.

A keyboard-only traversal uses Tab, native tab-list arrows and shortcuts rather
than programmatic focus or pointer clicks. It first reproduced Escape leaving
focus on the timeline when Next transition was disabled; Escape now returns to
an enabled control, keeping `/` usable at the completed head.

## Qualification

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features --no-fail-fast` | **1,278 passed, 0 failed, 0 ignored**; 38 result summaries including empty targets/doc-tests |
| `cargo test -p rastok --all-features ui:: --lib` | 20 passed after the CSP/favicon fix |
| `cargo doc --workspace --no-deps` | succeeds |
| core `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf`, `riscv32imac-unknown-none-elf` | all three succeed |
| SVG layout | 13 passed |
| isolated Chrome, including console errors | **24 passed, 0 failed, 0 skipped** |

The final JavaScript-only Escape correction was rebuilt and tested through the
complete browser suite, including the new keyboard-only traversal. Measured
playback intervals are 1,002 ms at 1 event/s and 252 ms at 4 events/s. No user
browser profile or pre-existing debugger session was used by these checks.

Logs and captures from this local run are under
`/tmp/rastok-ui-explanations.7EgVLI/`. They are local evidence, not repository
assets. The isolated Chrome harness captures B0, B1, B8 and the completed
transition at 1352×760, plus B21 waiting, closed/open decisions, assertion
differences, instance comparison, light mode and a 700×760 narrow view.

One earlier browser pass timed out starting its first CLI within the existing
30-second limit; the remaining 22 tests passed. The unchanged startup limit
passed on retry. No host setting or timeout was changed to conceal that result.
The installed in-app browser bootstrap remains unavailable, so qualification
uses the repository's isolated Chrome harness, not a personal browser profile.
No Claude/Fable process or independent reviewer was invoked.

Echo, JSONL and gauge semantic hashes were checked through the real CLI and
remain `5ffba3ea…`, `2f89c8b5…` and `0d3ad43d…`; the full suites enforce the
existing byte-pinned witnesses. The embedded spike was not remeasured in this
presentation pass; the three core `no_std` targets were checked.

## Follow-up to the independent UI retest

The retest confirmed the explanatory surfaces and identified the incorrect
`1 transitions` label. A browser regression first failed on that exact text; the
presentation now says `1 transition`. There is no execution or projection change.

The remaining manual checks use existing scenarios, not new runtime behavior:

- **Failed assertions:** launch
  `target/debug/rastok simulate crates/rastok/tests/fixtures/ui-explanation.rkg --ui`.
  Finish `inspect.mismatch`: the Assertion tab must show `data.value`, observed
  `7`, expected `8`. Select `inspect.unwanted` and Finish: `expect app.out == nothing`
  must show the one offending envelope. These failures are intentional fixtures.
- **Fresh keyboard session:** launch
  `target/debug/rastok simulate examples/keyed-retry.rkg --ui`, or use Reset in
  the existing debugger. Reloading the browser does not reset the simulation.
  With focus outside an input, `a` moves LIVE transition 0 to 1, then `n` moves
  LIVE transition 1 to 2. The browser regression starts its own CLI process.
- **Mailbox preview:** at retained transition 8, hover a nonempty `≡ 1` badge.
  The preview shows queued DATA without a click, changing inspector tabs or
  moving the selected transition. The test exercises pointer hover and keyboard
  focus separately.

The follow-up captures are named `assertion-difference`, `assertion-unwanted`,
`fresh-shortcut-a`, `fresh-shortcut-n` and `mailbox-preview`.

Follow-up results: 24 browser tests passed, no skipped test, browser exception or
console error; 20 Rust UI tests and 13 SVG layout tests passed. The embedded CLI
was rebuilt; JavaScript syntax checks, `cargo fmt --all -- --check` and
`git diff --check` passed. The five follow-up captures were visually inspected.
Logs and captures are under `/tmp/rastok-ui-retest.gN3oLG/`; the expected red
regression is `regression-before.log`, the green suite is `browser.log`.
The workspace-wide Rust suite, clippy, documentation and embedded targets were
not rerun for this JavaScript-only correction; their table above belongs to the
preceding qualification, not this retest. Existing user sessions were untouched.

## Limits kept visible

- An authored `advance` can still retain several Execution transactions inside one
  UI transition. Changing that grain requires the controller/Lab seam, outside
  this brief. This pass does **not** close the outstanding M1.9 stepping gate.
- Unknown predicate evaluation is not a failed predicate or zero matching
  envelopes. Arbitrary assertion expressions do not gain invented field diffs.
- A Session handshake whose opening delivery is no longer inspectable is not
  projected without its exact correlation. Later message deadlines cannot stand in
  for that opening. Pending items are obligations, not a predicted execution order.
- Comparison aligns retained transition ordinals; equal columns do not prove
  semantic equivalence. A missing transition remains unavailable.
- No editing, branching, scenario authoring, world rewind, live-run debugger or
  terminal was added. Independent usability review remains in the
  [release inventory](release-qualification.md).
