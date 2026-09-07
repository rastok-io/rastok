# R9 — migration and qualification

> **R0–R9 implementation and migration qualified, 2026-09-07.** The identity
> inventory in §1 was written before the single v16 package blessing. Earlier
> red cutover checkpoints are not the final gate. The real-browser pass remains
> an explicit external release qualification (§6), not a claimed result.

## 1. Identity inventory before blessing

The categorical movements are those allocated in
[R4](rework-r4-plan.md) and [the R0 ledger](rework-r0-identity.md).
All packages move once from v15 to v16. No numeric tag changes meaning.
The candidates below were decoded and independently validated before their
hashes were compared. Sizes exclude the package header.

| Witness | Body v15 → v16 | Semantic hash v15 → v16 | v16 body checksum |
|---|---:|---|---|
| echo | 907 → 939 B | `e50026ee…` → `5ffba3ea18c52a98a538a63ed0a7bf23cd3b4be0fa1b7d7acc51ba85dd73cfa3` | `f20579985db83680dfb5f27fe3d0e166600a43e9bf389cdf2e350886e2d68071` |
| JSONL | 951 → 983 B | `036f7ea5…` → `2f89c8b5fa22f7fbe489d6aa770d517c3c9db8f92e850028c1e2bb1280efb2fc` | `98ea0ce81a78671ff9c39e474018f5a92f2c923aa5bd9ad350724158b444e72f` |
| gauge | 243 → 249 B | **unchanged**, `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `36007746ece8b12cef5d4a9bd4f5aefb7b58acbf81f39158c675be7d6d02c2cd` |

Echo and JSONL carry `rastok/process@2`, explicit output dispositions and the
canonical Dispatch representation. Gauge has no Connector or Loop; its
container changes, not its meaning. Source spelling `.` → `in` / `out` /
named boundary is not an additional semantic change.

The four pinned network-contract source witnesses move for the same contract
and disposition reasons, without changing their request settings:

| Contract | Hash v15 → v16 |
|---|---|
| HttpClient with typed headers | `c66bd9da…` → `5760cb7523d7f4c8e2e39f766fa9d16e95cf6bcb8781a20ad76606ab42ef4cfd` |
| HttpServer ack | `b83fcf2d…` → `250183f684f79f4bfe8f422dfb017ebfbf180ee582cdca231925867047ec58e6` |
| WebSocketClient | `d4f92e15…` → `82a9e662475c271982710040e5c9b2da3c040a7ee2592f69ccbcc683f520411f` |
| WebSocketServer | `d36bcbf4…` → `bb0ff303266e39c9066187e86e4f518e6581944d251d9e12a3f9b308d6791820` |

The HttpClient default-settings witness moves `481662aa…` →
`2994c63ad59fee2e2b764f3528ede754001da648f3c5b5be0a7f1e837d65287d`;
omitted and explicit defaults still have identical plans and hashes.
The M0 Process probe moves `2933618d…` →
`d8c35016ba0d9a1c8a6b57e1abd83192840a5e9c11c63abd7efc977e365f126f`.
File's `@2` contract is independently pinned in the shared core contract
registry and exercised by its conformance and Loop qualification tests; there
was no File package golden to re-bless.

The measured causal witnesses are **not re-blessed**: echo's three seals
`21de0053…`, `f76107eb…`, `4f4950c1…`; JSONL `eb8aeb71…`; gauge
`6f463270…`. Their existing byte-for-byte tests must pass unchanged.

## 2. Source and evidence migration

The boundary-spelling twin test passed before the migration refusal was
enabled. Its explicit spellings remain pinned at `61847c23…` (conventional
ports) and `87ee3cbb…` (named ports). Old spellings now produce
`E-WIRING-MIGRATION` with the exact replacement. Full-envelope `out = .`
is unchanged. Examples and CLI fixtures use explicit boundary names. The
benchmark README freezes previously used prompt cards: v1/v2 are therefore
preserved, and v3 is a new card for v16 instead of rewriting prior-wave inputs.

The `minimal.ast` parser snapshot changes source spans and boundary path
syntax after the source migration; this is an AST conformance movement, not
an additional package or semantic movement.

Loop Graph fixtures now use explicit deciding Operators and manager commands.
The original 17 Loop-control tests and 74 machine tests were migrated and
passed before removal of the obsolete r1-only journal test. The r1
`loop_progress_is_a_close_once_journal_fold` test is retired because that
production protocol and RAM index no longer exist. Current r2 fold tests
verify command production, consumption, duplicate refusal, expiry and recovery;
the r2 golden test pins key 55 and absence of retired key 53. Domain separation
against retired identities is still tested without retaining executable r1 code.

## 3. Visual debugger boundary

R9 resumes UI alignment, not editor development. Only `simulate --ui` is
admitted; `run --ui` refuses before compilation, binding or effects. Real-I/O
experiments remain a separate named gate. The deterministic simulation can be
inspected backward/forward using retained boundary frames; no branching or
input editing is necessary. Reset creates a fresh simulation session at B0,
and previous session snapshots remain inspectable without replaying effects.

The former hosted-clock pause test is retired with the removed `run --ui`
controller; simulation time advances only through Lab actions. The branch
reconstruction test is retired with branch authoring. Timeline tests must
instead prove immutable historical inspection, reset isolation, retained
forward playback, and absence of those mutation commands.

## 4. Final gates

This is the closure matrix at `4dbb7a6`. The post-review parser correction and
its rerun checks are recorded separately in §7.

| Check | Executed result |
|---|---|
| `cargo test --workspace --all-features` | **1,256 passed, zero failed, zero ignored** |
| `cargo test --workspace` | **1,197 passed, zero failed, zero ignored** |
| `cargo fmt --all -- --check`; `git diff --check` | pass |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo doc --workspace --no-deps` | pass |
| core no-std: `thumbv8m.main-none-eabihf`, `thumbv7m-none-eabi`, `riscv32imac-unknown-none-elf` | all three pass |
| Embedded SVG layout tests, `node --test crates/rastok/src/ui/graph_layout.test.cjs` | **13 passed**; JS syntax checks pass |
| Public CLI UI tests after the reset-history regression | **17 passed**; immutable old session, new live B0, rejected branch command and identical stepped/continuous histories |
| Single package blessing, then ordinary witness tests | echo / JSONL / gauge v16 accepted and pinned; causal goldens unchanged |
| Five `examples/*.rkg` plus checked WebSocket gateway | all compile; embedded scenarios pass where present |
| Twenty benchmark references against their independent referees | all pass; T11/T20 explicitly expose the promised timeout on the App boundary |
| Benchmark runner tests and v3 canned rehearsal | **4 tests passed; 3 tasks passed**, two after the expected corrective attempt |
| Independent HTTP loopback probes | Caddy, NGINX, curl and nghttp2 markers all pass |
| Independent WebSocket loopback probes | Node `ws` and hyper-h2 + wsproto markers both pass |
| Cortex-M33, tracked v16 echo package | `SPIKE_OK`, **111,452 B** peak heap, **19,620 B** below 128 KiB |

The embedded measurement retains all 67 records and opens at 6,440 B. Relay
average/minimum/maximum is 1,879 / 1,823 / 2,065 ticks against 39,999 ticks
for 2,000,000 exact calibration instructions: about 103.3 k worst-case
instructions against 150 k. Text 859,004 B + rodata 68,056 B = **927,060 B**,
under 1 MiB. No gate was raised. These are the existing non-Loop relay and
linked-code measurements, not peak-memory qualification of a Graph Loop or
recovery workload. See [the full spike ledger](spike-rp2350-emulated.md).

The WebSocket probe's first two startup attempts ended before a listener was
available. A later CLI rerun also missed its five-second startup bound: the
child was verified still alive and listening afterward. These are not counted
as passes or attributed to a protocol defect without evidence. The launchers
now preserve server stderr on startup refusal, and the CLI test stops/waits
for a late child instead of leaving it orphaned. A process sample during the
rerun showed `_dyld_start` before Rust execution. The timed HTTP/WebSocket
CLI probes and WebSocket launcher now validate their source with public
`check` before starting their I/O budget. No protocol deadline is raised.
The final interop probe produced both success markers; workspace qualification
uses one built CLI without concurrent feature-changing builds.
[Release qualification](release-qualification.md) records the pinned
independent implementations, including still-open HTTP/3/Autobahn gates.

## 5. R0–R9 closure map

| Gate | Delivered authority and evidence |
|---|---|
| R0–R2 | normative algebra, host Contract tables, three reference traces, repository corrections, supersession and identity inventories |
| R3 | Extension/Contract vocabulary, one handler per trigger, Dispatch, `where` desugaring, sets, case routes, multi-emission and named migration refusals |
| R4 | one categorical v16 package, six `@2` Contracts, independent registry-free validators, retained r2 commands/context and timer deadlines allocated before blessing |
| R5 | explicit dispositions and re-derived productibility; actual App boundary only; no implicit root sink or name-based `err` policy |
| R6 | typed closed/open Dispatch, atomic selected fan-out, deterministic ordered multi-emission, false join admission preserving candidates |
| R7 | exact opening admissions, bounded manager commands, original deadlines, run-local quiescence and journal-only spatial/exchange reconstruction |
| R8 | Operator / Graph / outbound Call Loops, nested and adjacent runs, proof-only Graph/Operator profiles, annotation propagation and inner-first window closing |
| R9 | source/fixture/benchmark/document migration, one enumerated package blessing, unchanged unaffected witnesses and simulation-only debugger aligned to actual v16 topology |

Recovery regressions include every committed prefix across command production
and consumption, sibling Call settlement, nested expiry, joins, original
timeouts, Closing and inbound exchanges. Relevant suites are
`v16_loop`, `v16_loop_composition`, `v16_graph_calls`, `v16_recovery`,
`v16_exchange_recovery`, `v16_loop_policy` and `loop_control`. A committed
producer is never re-executed to recover an unconsumed command; reconstruction
does not restart an old Driver effect. A window child's stop emission reaches
its live downstream sibling before the parent stop.

The [R0 reference traces](rework-r0-reference-traces.md) now explicitly name
their executable record-granularity corrections: opening context is installed
at manager admission, and pure Call preparation plus open is not a separate
request-consuming transaction. The real completion consumes that request once.
Prefix equivalence tests compare actual records, not the sketches' row numbers.

Runtime/HIR no longer contain Graph application handlers, `ChildPort`
activation, `RootSink`, `UnwiredPolicy`, `LoopWork` or an r1 Loop journal index.
Parser-only recognition of removed spellings remains solely to issue named
refusals; retired wire tags remain rejected, not reassigned. No test is ignored
to obtain the green matrix. The retired tests in §2–§3 name removed behavior
and their current replacements.

The normative specifications, README, architecture overview and white paper
describe the same model. Old cutover reports and pre-v16 integration sketches
are explicitly historical. They are not executable examples or current syntax
cards; R9 does not claim the prospective Teams/service/guest integrations.

## 6. Named limits and external qualification

- The Loop proof admits an unconditional same-context join only when each
  application supplies exactly one candidate per input. Retained/expiring or
  guarded joins and descendant lifecycle-dependent Loop bodies still refuse
  at check; ordinary joins/windows and their recovery are qualified outside
  that proof profile. The proof refuses above 1,024 distinct outcomes rather
  than approximating. See [Loop §16](../spec/loop.md#16-deferred-by-name).
- Session/Event/inbound-Call Loops, Saga execution, detached obligations and
  opaque/affine/lexicographic termination proofs remain named future work.
  Reconstructing a retained Session is not physically reattaching a lost socket.
- Browser qualification was attempted but the installed controller failed
  before connection because its worker referenced an absent older module.
  **No screenshots, keyboard pass or visual/UX approval are claimed.** The
  HTTP/controller/projection and SVG geometry tests are separate evidence;
  the browser pass remains in the single release inventory.
- The debugger remains simulation-only and read-only with respect to source
  data. Real-I/O experiments, authoring, branches and the proposed terminal
  debugger are not reopened by R9. Neither OTLP export nor public-release
  qualification is implied by the v16 rework closure.

## 7. Final-review correction — filtered Dispatch

The review of `4dbb7a6` found that `where P match ...`, already specified by
the grammar and R0 §6.1, was swallowed as one guard expression. The hosted
expression framer now leaves the top-level `match` keyword to the handler
parser, without treating members such as `data.match` as delimiters.
No runtime, core, package-format or identity recipe changes are needed.

Parser regressions cover open/closed Dispatch after a guard, join bounds,
member names, strings, comments, exact guard spans and a missing arm arrow.
The `where_dispatch` integration suite verifies narrowing, identical plan
bytes/hash/journal against explicit nested Dispatch, false one-port filters
consuming once, and false join guards retaining both candidates before later
matches. Unbounded content joins still refuse with their retention diagnostic;
a malformed guard remains an expression error rather than a bare-statement error.

The Budget design memory is explicitly pre-v16 syntax, the R0 witness table
links to the blessed §1 ledger, and the release inventory tracks the exact
remaining embedded code-size margin before Saga. The R9 spike figures remain
the prior measurement, not a new embedded workload claim from this hosted-only
parser correction.

Post-review verification: **1,261 tests passed, zero failed, zero ignored,
37 suite results** with `cargo test --workspace --all-features --no-fail-fast`.
Formatting, diff checks, all-target/all-feature clippy with warnings denied,
workspace documentation and all three no-std target checks pass. Two public
CLI scenarios also pass: one-port filter/dispatch and retained join candidates.
No golden changed. The default-feature, independent interop and embedded-spike
measurements in §4 were not rerun for this correction.
