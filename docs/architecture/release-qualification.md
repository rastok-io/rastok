# Release qualification

> This is the single inventory of evidence deliberately kept outside the
> hermetic Rust workspace gate. A milestone report may link here, but may not
> silently omit an open row or turn it into a product-readiness claim.

## Executed on the reference host

| Surface | Independent implementation | Evidence | Last result |
|---|---|---|---|
| HttpClient HTTP/1.1 | Caddy 2.11.4 | [`spikes/http-interop`](../../spikes/http-interop/) | `CADDY_HTTP1_CLIENT_OK` — rerun 2026-09-07 |
| HttpClient HTTP/1.1 | NGINX 1.31.4 | [`spikes/http-interop`](../../spikes/http-interop/) | `NGINX_HTTP1_CLIENT_OK` — rerun 2026-09-07 |
| HttpServer HTTP/1.1 | curl 8.7.1 | [`spikes/http-interop`](../../spikes/http-interop/) | `CURL_HTTP1_SERVER_OK` — rerun 2026-09-07 |
| HttpServer HTTP/2 | nghttp2 1.70.0 | [`spikes/http-interop`](../../spikes/http-interop/) | `NGHTTP2_SERVER_OK` — rerun 2026-09-07 |
| WebSocketServer HTTP/1.1 Upgrade | Node `ws` 8.21.3 | [`spikes/websocket-interop`](../../spikes/websocket-interop/) | `NODE_WS_OK` — rerun 2026-09-07 |
| WebSocketServer HTTP/2 Extended CONNECT | hyper-h2 4.3.0 + wsproto 1.3.2 | [`spikes/websocket-interop`](../../spikes/websocket-interop/) | `H2_EXTENDED_CONNECT_OK` — rerun 2026-09-07 |

These are blocking loopback probes with finite deadlines. Package retrieval is
explicit, pinned and separate from `cargo test`; it is not a hidden network
dependency of normal development or CI.

## Public destinations and opening gate

Yoann confirmed these destinations on **18 September 2026**:

| Metadata | Confirmed URL |
|---|---|
| Repository | `https://github.com/rastok-io/rastok` |
| Homepage | `https://www.rastok.com` |
| Documentation | `https://docs.rastok.com` |
| Releases | `https://github.com/rastok-io/rastok/releases` |

Both crates inherit the three standard Cargo URL fields from the workspace.
Their readmes and the root README carry the release link; the latter is
included in binary archives by the prepared artifact workflow. The repository
is to be created privately and opened only on Yoann's instruction on
5 November. The unowned bare GitHub handle is not a release destination.
`publish = false` remains in force.

**Still open:** before crate publication, `docs.rastok.com` must resolve to
usable documentation. Prefer the minimal Starlight quickstart and six gallery
chapters on 5 November, with the rest following in November. The fallback is a
301 to the actual gallery page on `www.rastok.com`, replaced by the documentation
site in November; the separate site repository must confirm that target path.
The bare `rastok.com` domain is intended to redirect with 301 to `www.rastok.com`.

This records addresses and a handoff, not live availability evidence. No DNS
change, deployment, remote workflow, push, tag or publication is performed or
authorized by this metadata preparation. The site handoff is tracked in
[`docs/site/README.md`](../site/README.md#public-destination-handoff--18-september-2026).

Private reporting and contribution controls have a separate
[owner checklist](github-opening-checklist.md). The local DCO check and
templates are prepared, not activated GitHub settings. `contact@rastok.com`
is the planned address for Yoann; mailbox/forwarding activation and an external
receipt test remain necessary. Main protection, labels and private vulnerability
reporting are unconfigured here. The full-history DCO audit identifies one
unsigned historical commit requiring a maintainer decision, not automatic
rewriting. These gates must close before public participation opens.

## Open qualification inventory

**Current representation — 15 October:** package 21 and diagnostic recording 4
are described by the [Node realignment inventory](node-realignment-identity.md).
Only the package representation changes; semantic hashes, journal identities,
causal traces and recording tuples remain unchanged. The
[lot 1 qualification](node-realignment-qualification.md#lot-1-closure--14-october-2026)
records lot 1's closure: 1,591 passing workspace tests in 61 groups, no failures
or ignored tests, strict Clippy, rustdoc, three-target `no_std` and embedded
checks. The local browser pass remains separately dated in that report.
D-2026-10-14-1 resolves the depth boundary with a 256-level reader budget and
unchanged expression/type bounds of 128. The accepted level-255 combined
package uses 79,168 B of decoder stack under Cortex-M33 QEMU. Inspection-module
separation and the hosted Worker remain lot 2.
This does not qualify release archives, constitute an independent browser
review, or close any external opening gate below.

**7 October dated qualification:** package 20 and diagnostic recording 4
had one reviewed reference update in the [human-review identity inventory](human-review-v20-identity.md).
That ledger records the workspace and three-target `no_std` evidence at that checkpoint.
The final all-feature workspace run passes **1,571 tests, zero failures or
ignored tests, 61 groups**; fmt, strict Clippy and rustdoc pass as well.
Both fixed Cortex-M33 workloads pass: echo uses 112,656 B peak heap and
973,028 B code; Retry with full recovery uses 130,560 B and 1,041,440 B.
Retry's margins remain only 512 B heap and 7,136 B code. M2 must reclaim
bytes before growing the core; neither threshold has been relaxed.
This cutover does not rebuild release archives or perform a new independent
browser review. The dated artifact/browser evidence and open rows below must
not be presented as qualification of freshly published binaries.

**18 September dated qualification:** native Retry, two-field measurements
and the Responses adapter share one v19/v3 blessing, documented in the
[C4 ledger](retry-c4-qualification.md). The final ordinary workspace run has
1,545 passing tests, zero failures/ignored; earlier listener-start failures
and unchanged passing reruns remain recorded. Thirty-five isolated Chrome
tests pass. Fresh flagship stills at 0/20/34 (1352×760) and a 120-frame,
30.000-second GIF were visually inspected locally. All 27 gallery scenarios
match their goldens. This closes local implementation/fixture work, not the
independent review or release authorization below.

Both macOS release binaries and extracted crate builds are requalified;
archives and checked SHA-256 values are local under
`/tmp/rastok-m1-artifacts.orJti6/dist/`. No Linux target/container/cross-toolchain
is installed here and no remote workflow is invoked. Public URLs are now
confirmed and prepared in local metadata; their availability is an opening
gate below. The two unaided quickstart trials and independent browser review
have not been performed. The artifact builds above precede this URL-only
metadata update and are not claimed to contain it.

The 10 September M1 opening brief makes the debugger visual/interaction pass
**blocking for 5 November**. The three protocol/contribution rows are
**non-blocking, still unqualified**: they must remain disclosed in the README
and release evidence. This reclassification is not a passing result.

| Qualification | Opening gate | Required environment | Closure evidence |
|---|---|---|---|
| ngtcp2 client against live Rastok HTTP/3 server | Non-blocking; open | current C++23 compiler, nghttp3, event loop and explicit supported TLS backend | pinned command, versions and successful terminal transcript |
| Official Autobahn RFC 6455 client and server roles | Non-blocking; open | container-capable host for the maintained frozen PyPy 2 runner | client/server HTML reports linked from the release record |
| Marginal Connector implementation benchmark | Non-blocking; open | one new representative Connector after M1.6, measured from empty contract-specific module to conformance | repeatable timing and scope record in the release report |
| M1.9 hosted UI visual and interaction pass | Blocking; open | a working real-browser controller on the reference host | initial-pause/control/reconnect/reset/completed-state transcript; read-only simulation back/forward and reset-session isolation; buffered Replay at 1/4 transitions per second; mailbox/instance inspection, zoom/pan and splitter pass; keyboard pass; same-viewport 1352×760 captures at B0, B1, B8 and the completed transition plus a narrow screenshot linked from the M1.9 report |

R9 attempted the browser pass on 2026-09-07. The installed controller failed
before connection: its worker referenced an absent older browser-service module.
That R9 attempt produced no visual/keyboard evidence. The subsequent
[UX refinement](m1.9-ux-qualification.md) runs isolated headless Chrome through
real-browser tests, with keyboard checks and desktop/narrow captures. The
[independent Fable audit](m1.9-fable-ux-audit.md) then confirmed usable navigation
but found an open `advance` stepping gap and remaining graph/inspector UX work.
Neither the automated checks nor that review close the M1.9 row above.

The [2026-09-08 explanatory projection pass](simulation-debugger-explanations.md)
adds retained decision/causal explanations, timer and assertion evidence,
keyboard navigation and read-only comparisons. Its browser checks include
console errors and five examples in LIVE, HISTORY and REPLAY. The existing
authored-`advance` stepping gap remains explicitly outside this presentation pass.

The subsequent user-supplied independent retest of `ui_tour.two_tenants`
confirmed the transport vocabulary, prefix-relative Loop/delay dial, selected
match arm, causal links, scenario navigation and pending obligations (9/10).
It did not exercise a failing assertion, fresh-session `n`/`a`, or mailbox hover.
Commit `fd525d7` adds focused regression evidence for those paths and singular
transition wording; automated evidence is not a substitute for their independent
visual retest. The open qualification row remains open.

The [documentation audit](documentation-audit-2026-09-09.md) reruns the hermetic
workspace and executable examples, not the external protocol probes or a new
bare-metal measurement. It additionally identifies a pre-release checker/core
productibility defect for bounded-join expiry. The
[opening correction and its regression evidence](opening-showcase.md) track
its closure separately; the otherwise green audit tests did not waive it.

Public Internet probes remain scheduled canaries, never replacements for these
local qualifications. The gallery, website and demonstration are M1
deliverables tracked by the roadmap rather than protocol-conformance evidence.

### 10 September local continuation

The two HTTP/3 workspace probes were reproduced failing on a host resolving
`localhost` to IPv6 first while their fixtures listened only on IPv4. Their
listeners now follow the same local resolution as the client. Both focused
probes pass with DNS-name certificate verification still enabled; no physical
Driver, TLS policy, semantic Contract or golden changed. This corrects local
test setup, not ngtcp2 qualification or a claim of multi-address connection
fallback in the Driver.

The isolated Chrome harness passes 30 tests, including all six webhook
scenarios in LIVE, HISTORY and REPLAY with exact costs and no email or bearer
disclosure. Fifteen layout and three editor lexical tests also pass. The
integrated browser still fails before connection, so its independent pass
remains unclaimed. The flagship's initial captures exposed crowded titles,
badges and labels. A rendered-geometry regression reproduced 23 text overlaps
before correction. Separate scope bands now reuse rank columns, text and
badges have reserved space, and the actual cubic curves avoid label/header
obstacles. The rebuilt binary passes the new regression at transitions 0,
1, 8 and completed at 1352×760, plus narrow width: interface-sized titles,
no overlapping text or curves crossing labels, the full desktop graph in its
canvas, and HISTORY clear of zoom controls. Four-digit mailbox badges have a
separate layout regression. The captures were visually rechecked locally;
they remain diagnostic, not approved publication assets. Evidence:
`/tmp/rastok-graph.IgAGKl/browser.log`, `layout-editor.log` and
`screens/webhook-geometry-{initial,1,8,completed,narrow}.png`.
The independent browser audit, the remaining controller granularity gap,
approved captures and GIF were still open at that checkpoint; these totals
did not waive those gates.

### Publication assets — 14 September

The three real flagship PNGs and 30-second GIF now live in
`docs/site/assets/` and are linked from the chapter and README. They show
`webhook.timeout_then_success`, 1352×760, transitions 0, 18 (pending delay),
and 29 (success after two applications). The machine-readable capture manifest
maps all 120 GIF frames to retained prefixes; `ffprobe` verified 30.000 seconds.
All three PNGs were visually inspected locally. Reproduction:

```sh
RASTOK_PLAYWRIGHT=/path/to/playwright \
RASTOK_CHROME=/path/to/chrome \
RASTOK_UI_ASSETS=/path/to/an/empty/output-directory \
node crates/rastok/src/ui/capture-showcase.cjs
```

This uses installed browser/FFmpeg tooling, not a frontend dependency or an
independent audit. The in-app browser provider was unavailable; the repository's
headless Chromium path produced these assets. Yoann's publication review and
the independent browser/controller gate remain distinct and open.

The gateway/documentation follow-up passes **31** local Chrome tests,
**15** SVG/layout tests and **5** editor tests (three lexical, two real
TextMate/Oniguruma). The new gateway is exercised in LIVE, HISTORY and REPLAY
without a process or model invocation. This is automated local evidence, not
the separate independent audit. Its batch/step regression additionally compares
complete canonical journals, including resident multi-frame replies.

### Fine-grained stepping and local release preparation — 14 September

The six remaining implementation/UX items from the 7 September audit now
have local regression evidence: resumable logical-time advance, original
App-output inspection, previously qualified graph spacing, folded envelope
history and readable META keys, retained-route instance labels, and centred
toolbar zoom with a fit-relative percentage. The **33-test** browser suite
passes against the release binary, including console checks, keyboard paths,
desktop/narrow layout, history and replay. The original grouped-time B21–B22
positions are no longer authoritative: waiting is selected from the actual
Loop fold. This does not claim an independent review of the corrected build.

Publication assets were regenerated from the same real browser at 1352×760.
The current flagship stills are at transitions **0, 23 and 39**; the GIF has
**120** retained-prefix frames and **30.000 seconds**, verified with ffprobe.
They replace the earlier 0/18/29 presentation, not any causal golden. The
manifest records every displayed prefix. The three views were inspected
locally; the independent visual gate above stays open.

Both `0.1.0-alpha` crate archives build after extraction. Because core is not
published yet, hosted-package verification uses the extracted core archive
through a command-local Cargo patch, not a registry publication. A direct
offline workspace package attempt correctly refuses that unpublished
dependency. License files, readmes, starter source and embedded UI assets are
included. At that checkpoint Cargo warned about missing public repository/
documentation URLs; the 18 September metadata handoff above resolves their
choice, not their deployment.
`publish = false` remains in force; no release tag has been created.

macOS arm64 and x86-64 optimized binaries are built with
`hosted-ui,hosted-websocket`, excluding HTTP/3. Both create and simulate the
starter graph. The Intel binary also passes echo under Rosetta; that is not
a native Intel-host qualification. Archives and SHA-256 values are local
evidence, not announced downloads. Linux cannot be qualified on this host
without a target environment; the prepared manual build workflow has not
been run remotely. The [two external quickstart trials](quickstart-trials.md)
remain unperformed.

## Retry journal-scan debt before M2

The 18 September Fable prereview identifies repeated full-prefix traversal in
`runtime/vm/retry_manager.rs`: `synchronize_retry_closures` scans when Calls
are open, `next_dated_recurrence` scans for retained creation order when both manager
folds exist, and `step_retry_manager` scans when work is ready. `MemJournal`
also performs linear `find_call` / `find_envelope` lookups. This is a structural
cost analysis, not a measured latency claim: S eligible steps over a prefix
of up to J records can cost O(S × J), hence O(J²) when both grow together.

This remains a named M2 scalability debt, not a newly failed M1 gate. Before
durable storage, qualify derived indexes for exact Calls, envelopes and the
winning retained deadline creation order, including recovery reconstruction and
bounded-memory costs. Do not turn a retained-prefix traversal into disk I/O
on every step or replace the journal's tie-breaker with RAM/scheduler order.
No optimization, index format, journal fact or golden change is made in this
documentation follow-up.

## Embedded code-size watch before Saga

**Current measurement — 7 October:** both fixed Cortex-M33 workloads pass.
Echo uses **112,656 B peak heap / 973,028 B code**, leaving **18,416 B /
75,548 B** under 128 KiB / 1 MiB. Retry with full recovery uses **130,560 B /
1,041,440 B**, leaving **512 B / 7,136 B**. The
[human-review inventory](human-review-v20-identity.md) records the executed
commands. No core growth is accepted without reclaiming bytes first. These
are workload measurements, not bounds for all admitted programs.

**Dated measurement — 18 September (v19):** the unchanged Retry/full-recovery
package passes QEMU with 17 records and **1,041,724 B code / 130,560 B heap**.
That leaves **6,852 B / 512 B** under the existing 1 MiB / 128 KiB gates.
The freshly blessed echo image passes with 67 records, **973,524 B code** and
**112,656 B heap**. These are workload-specific limits; maximum-size metadata,
JSON or retained-batch workloads are not thereby qualified. Further core
changes must remeasure before growth. The [C4 ledger](retry-c4-qualification.md)
records commands/evidence. Measurements below are historical checkpoints.

The 16 September **unblessed v19/v3 candidate** has its own
[Retry/full-recovery qualification](retry-c3-implementation.md#42-shared-admission-and-bounded-collection-construction-16-september).
Shared admission code and direct collection construction bring that image
from 1,104,268 B to **1,040,824 B**, leaving **7,752 B** below 1 MiB. Its
three-attempt, 16-frame-per-channel workload still peaks at **130,560 B**,
only **512 B** below 128 KiB. Reference echo measures **972,656 B** of code
and **112,656 B** peak heap. Both QEMU workloads pass their unchanged gates;
this is not a blessing of the candidate format or an embedded bound for
unmeasured annotation growth, JSON bodies or storage-refusal batches.

The subsequent [declared-annotation checkpoint](annotation-types-m1.md#11-declared-descriptive-applications-16-september)
remeasures that same Retry package at **1,046,444 B** of code and unchanged
**130,560 B** peak heap (`SPIKE_OK`, 17 records). Only **2,132 B** of code
margin remain. This checks the new decoder/validator on the fixed workload,
not the additional retention of large metadata tables or future business
measure bindings. Those workloads still require measurement before closure.

The [additive business binding](annotation-types-m1.md#13-additive-business-binding--implementation-inventory-16-september)
subsequently measures **1,048,232 B** (`.text` 964,612 + `.rodata` 83,620),
with **344 B** of code margin and unchanged **130,560 B** heap peak. QEMU
retains the same 17 records and reports `SPIKE_OK`. The initial implementation
exceeded 1 MiB; shared definition-check storage, allocation-free standard-schema
comparison and in-place canonical annotation sorting remove that excess.
The fixed unannotated Retry image exercises validation/recovery, not a
maximum-size annotation table or an embedded accounting query. Those profiles
remain unqualified; neither gate was raised and no golden was changed.

The [named monetary profile](annotation-types-m1.md#14-named-monetary-quantities-16-september)
then measures **1,048,344 B** (`.text` 964,620 + `.rodata` 83,724), **232 B**
below the same code gate, with unchanged **130,560 B** peak heap. The
definition-table walk was simplified first (576 B saved in isolation);
read-only schema validation no longer clones the type and annotation table.
The fixed Retry/recovery image again reports 17 records and `SPIKE_OK`.
This is not evidence for linked embedded accounting queries or maximum-size
metadata tables: those qualification gates remain open. No bound was raised.

The [17 September timer-order correction](retry-c3-implementation.md#8-mixed-timers-bounded-publication-and-exact-instances--17-september)
remeasures the unchanged Retry/full-recovery workload at **1,048,368 B**
(`.text` 964,644 + `.rodata` 83,724): +24 B, leaving **208 B** below 1 MiB.
Peak heap remains **130,560 B**, with **512 B** of margin. The same candidate
package hash, 17 records and `SPIKE_OK` are retained. Reference echo is
**980,172 B** of code and **112,656 B** peak heap, 67 records and `SPIKE_OK`.
These measured margins require remeasurement before further core growth;
neither workload establishes a bound for maximum-size annotated payloads.

The [projected JSON profile](openai-responses-hydration.md#implementation-and-qualification--17-september)
remeasures that exact Retry package at `.text` **964,764 B** + `.rodata`
**83,756 B** = **1,048,520 B**, **56 B** below 1 MiB. Peak heap remains
**130,560 B**, 512 B below 128 KiB; the same hash, 17 records and `SPIKE_OK`
are retained. The source/core attribute mirrors agree. This image includes
the independent schema/profile validation, not a hosted JSON response or a
maximum annotation-inspection workload. Further core growth requires a
measured reduction first or a separately approved bound change; neither
threshold has been raised. Evidence: `/tmp/rastok-json-profile.ygk4aj/`.

The [explicit-unit registry retirement](annotation-types-m1.md#16-explicit-unit-migration-inventory--17-september-before-implementation)
then removes the implicit-name lookup and its obsolete carrier/subset types.
The **same** 956-byte Retry/full-recovery package measures `.text`
**963,420 B** + `.rodata` **83,136 B** = **1,046,556 B**: 1,964 B recovered,
**2,020 B** below 1 MiB. Peak heap remains **130,560 B**, **512 B** below
128 KiB. The package hash, three Calls, 16 frames per channel, 17 records
and `SPIKE_OK` are unchanged. This is still the fixed Retry workload, not
maximum annotation storage or linked individual inspection. No gate was
raised. Evidence: `/tmp/rastok-measure-migration.tZmKT9/spike-fixed.log`
and `spike-size.log`.

The 14 September resident Process pass retains **111,732 B** peak heap on
the same Cortex-M33 echo workload, leaving **19,340 B** below 128 KiB.
`.text` **881,788 B** + `.rodata` **69,680 B** = **951,468 B**, leaving
**97,108 B** below 1 MiB. That is **408 B** above the two-budget shutdown
checkpoint, with unchanged heap. QEMU reports `SPIKE_OK`, 67 records and the
same echo hash. The Unix Driver is hosted-only; these measurements qualify
the core changes, not 64 resident children or arbitrary deployment workloads.

The gateway ownership correction was remeasured on 14 September: the same
**111,732 B** heap and **951,468 B** text/rodata, 67 records and echo hash.
All three bare-metal compilation targets pass as well; no gate was raised.

The `has` and resumable-time follow-up was remeasured on the same workload:
**111,732 B** peak heap (unchanged), `.text` **882,204 B** plus `.rodata`
**69,824 B** = **952,028 B**. Code grows **560 B**, leaving **96,548 B** below
1 MiB; heap still has **19,340 B** below 128 KiB. `SPIKE_OK`, 67 records and
the echo hash remain unchanged. The three no_std targets pass. No memory or
code-size gate was increased.

The [R9 Cortex-M33 measurement](spike-rp2350-emulated.md) retains **927,060 B**
of `.text` plus `.rodata` against the unchanged **1,048,576 B (1 MiB)** gate:
**121,516 B (118.7 KiB, 11.59%)** remain. This is the existing non-Loop relay's
linked image, not peak-memory qualification of a Graph Loop or recovery workload.
Before Saga or another core expansion, remeasure code size and heap first,
record the delta against R9 and identify the growing sections. A gate increase
requires an explicit decision; a green heap measurement does not substitute
for the code-size gate.

The [v17 host-terminal amendment](loop-host-terminal-v17.md) was remeasured on
2026-09-08 with the same echo workload: `.text` **860,700 B** plus `.rodata`
**68,156 B** = **928,856 B**, a **1,796 B** increase from R9. The unchanged
1 MiB code gate retains **119,720 B**; peak heap remains **111,452 B**, with
**19,620 B** below 128 KiB. `SPIKE_OK` retains 67 journal records. These remain
workload-specific measurements, not worst-case Loop or recovery bounds.

The 2026-09-09 bounded-join correction and diagnostic-reader seams retain
**111,452 B** peak heap on that same Cortex-M33 workload. The rebuilt image is
`.text` **860,404 B** + `.rodata` **68,172 B** = **928,576 B**: **280 B less**
than the v17 measurement, with **120,000 B** below the code gate. QEMU reports
`SPIKE_OK`, the same echo hash and 67 retained records; the worst relay sample
is 2,065 calibration ticks. This does not qualify the hosted recording codec
for bare metal: file I/O and the debugger remain outside `rastok-core`.

The 2026-09-10 exact-instance closure amendment (v18) measures **111,732 B**
peak heap on the same relay: +280 B, leaving **19,340 B** below 128 KiB.
`.text` **881,156 B** + `.rodata` **69,624 B** = **950,780 B**: +22,204 B,
leaving **97,796 B** below 1 MiB. QEMU reports `SPIKE_OK`, 67 records and
the unchanged echo hash; the slowest relay is 2,093 calibration ticks.
No gate is raised. This prices the linked implementation and the unchanged
relay, not an embedded workload executing/recovering closure commands.
