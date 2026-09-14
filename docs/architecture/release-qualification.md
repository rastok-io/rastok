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

## Open qualification inventory

The 10 September M1 opening brief makes the debugger visual/interaction pass
**blocking for 15 October**. The three protocol/contribution rows are
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
`webhook.timeout_then_success`, 1352×760, transitions 0, 18 (armed delay),
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
included. Cargo still warns about missing public repository/documentation
URLs; those must be supplied once the public destination is confirmed.
`publish = false` remains in force; no release tag has been created.

macOS arm64 and x86-64 optimized binaries are built with
`hosted-ui,hosted-websocket`, excluding HTTP/3. Both create and simulate the
starter graph. The Intel binary also passes echo under Rosetta; that is not
a native Intel-host qualification. Archives and SHA-256 values are local
evidence, not announced downloads. Linux cannot be qualified on this host
without a target environment; the prepared manual build workflow has not
been run remotely. The [two external quickstart trials](quickstart-trials.md)
remain unperformed.

## Embedded code-size watch before Saga

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
