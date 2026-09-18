# C4 — common v19 / recording-v3 qualification

18 September 2026. This ledger joins the previously inventoried Retry,
annotation and typed-JSON changes. It does not authorize a public release,
paid provider call or external audit. The [opening ledger](opening-showcase.md)
owns those remaining gates.

## 1. Inventory frozen before the single blessing

The [C1 allocation](retry-c1-identity.md), [example migration](retry-c4-examples.md)
and [two-field annotation inventory](annotation-types-m1.md#19-two-field-measure-migration-inventory--18-september-before-code)
are the complete movement set. No additional version, Contract major or
journal fact is allocated by this qualification.

The existing `package_ledger` example independently decodes the candidate,
compares its body to the stored v18 fixture and simulates its checked Lab.
The measurements below precede any fixture update; sizes exclude the package
header. The evidence directory is `/tmp/rastok-m1-measure.AsyHN2/`.

| Witness | Body v18 → v19 | Candidate semantic hash | Candidate package checksum |
|---|---:|---|---|
| echo | 939 → 964 B | `878a34b77a7ac6500b6452b903f633ba39848c47ed0ac387ce1597f78a785699` | `4186227832f5fd8c0d3d6adf205ae9c187fbf217c500310f31fb1dbb5e6a74c0` |
| JSONL | 983 → 1,008 B | `3f1b40ec3457d1b371289d9cac615df0394ef1dffe8b52c7f36424961a0dd064` | `9562d0b55923badb82264d6ac08817bfe8e280d6dda4412dc3a130f8d1432932` |
| gauge | 249 → 249 B, byte-identical | `0d3ad43d9fb09c8626146342f6a50d1f4c387a8de52a47a3134a408cf74cb22e` | `efb01e53be0025e53505f712cd930797bd4729bb101f791ca0bf365b8a1a4ddb` |

Echo and JSONL move for `process@4`, removal of `effect` and the explicit
plan-owned idempotence guarantee. Gauge has no Connector or annotation: only
its package version/checksum moves. Their causal traces are not in the
re-bless set. The echo roundtrip remains 8 records, seal `21de0053…`; JSONL
8 records, `eb8aeb71…`; gauge 4 records, `6f463270…`. Echo's filtered and
timeout scenarios remain `f76107eb…` and `4f4950c1…`.

The distinct M0 compiler probe moves from `d8c35016…` to
`3fe6e5319383eec3792f2981a40f9579fce96a4e1c3efe7b52b491dad932bc1a`
for the same Process Contract/default change. Representative network hashes
move for their exact `@3` Contracts, removal of the effect sentinel and,
for HttpClient, idempotence and typed `ServerError`:

| Fixture | Candidate hash |
|---|---|
| HttpClient | `a1bf3c54b12e0bb08795d67b2849560fa1b529ec97f802484d7578ef77134f56` |
| HttpServer | `de5b489357e12e52bbd7b536e133d103dc6e7430734fa65e476dfee38e99fcdf` |
| WebSocketClient | `e5d8c57b20427655842aa608f078858c744ede0a9648f3a4b881bb3c4b612020` |
| WebSocketServer | `37c7043e337fb958e7a743ddd1d9ad1693b1bf9d169297f2b882efbafe356667` |
| HttpClient effective defaults | `135282f162cbc350e091d230ff4d35077c920003a8f541c6173a47e671c1c661` |

The echo retained-record tuple digest becomes
`eba461cb2011177539b0a0a864ee01064f39097cace16dd3268b5523ce9e2078`:
recording v3 removes `Intent.effect` and retains exact Call closure time.
This is not a causal-seal change. Recovery still uses the original validated
plan, not a guarantee copied into an Intent.

`rastok/measure@3` and its two required text arguments replace both the old
six-field generic binding and compact cost tag 4. Measured numerical DATA,
declared types and original causal provenance stay exact. No cost basis,
currency conversion, aggregate, tariff execution or Budget policy is inferred.
The three package witnesses above are unannotated, so this final simplification
does not move them a second time.

## 2. Gallery traces and their reasons

| Trace | Records | Candidate seal | Reason |
|---|---:|---|---|
| two-tenants / `two_tenants` | 29 | `5da6e21cbc9c31dcec0cb47f681e85d70e6f33c9c93db700cfbb502b31eb1fc2` | Native Retry; no authored attempt field, one audit per request, Bob is never recalled |
| two-tenants / `lifetime` | 30 | `28b8b18fb75253db9eb6868f1a111e80534dcb2fd258ab1c9d4c819bbf0ef842` | Same migration; exact idle generation remains asserted |
| webhook / `nominal` | 18 | `fe27ba52836b5060ed21ab6369836f65b2d01e0330e17be0c3f5627959de9179` | Native Retry and original typed Receipt, no Loop state/extractor |
| webhook / `timeout_then_success` | 23 | `d261ea4b1575baf1ee6b8386309fa390eb0489d077f6fb3fb0972b39737bf46b` | Held timeout, original deadline, final publication only |
| webhook / `partner_dead` | 23 | `ffe3f847964b147a4dbae40736d7ec160cf2ffc85aeda813e44fb2b01a6aa941` | Last failure unchanged, no `LoopExpired` wrapper |
| webhook / `duplicate` | 35 | `b6804d852110f84f45d10fc690724683d073bb417b6db91f3d349b9a2242577f` | Native operation; explicit deduplication and idle retention preserved |
| webhook / `http_500` | 23 | `60427925392f11a772445aa3e506a806e20829d52dcd71468dec63ea67f6a807` | `ServerError`, held original charge, final-only delivery |
| webhook / `invalid_payload` | 9 | `2b9196216ce5d95a715793f0336631ca4e1edc6d94d521f3a987a6e6b897f232` | **Unchanged**: the invalid business payload never reaches the Call |

The last row corrects the preliminary C4 claim that all six webhook traces
would move. It already matches the stored golden and must remain untouched.
The first difference in each tenant trace is record 0, removal of the authored
`attempt` field. The five changed webhook traces first differ at record 7,
their outgoing delivery state. The public diff still redacts personal DATA;
classification is not bypassed to explain the fixture update.

The Responses adapter has six first-time goldens, not replacements for an
older executable HTTP example:

| Scenario | Records | Seal |
|---|---:|---|
| `two_sessions` | 31 | `410683daf1c6543ddff7ee6402895ee9eda74f4d68b44216b0caff902ad5f989` |
| `missing_usage` | 19 | `368617955fa96f6f1f51c22f205b3b3e5ae49855d3503df3e4fd149b606dde06` |
| `observed_zero` | 12 | `fe0307addd028e09ddb56a27df8f65ac6afa02d854904aaed4cb109af92351b7` |
| `schema_failure` | 10 | `c80f66ccd6242fb2872af5fb956e03968497b887db9a1071ebbfa039b01605e6` |
| `transport_failure` | 10 | `2382de8b0715e6136e37f516ab8067771ef1137c9c21049775448bce72be8032` |
| `http_errors` | 18 | `5437810490b743e54ab028631f1f02821df600e1f2680e36ff9e6fdc6bb2ce38` |

These are Lab outcomes. Local physical HTTP tests separately prove hydration,
wrong-type refusal, absent/null versus reported zero and secret-free retention.
No paid OpenAI call or inference idempotence is claimed.

## 3. Local qualification completed — 18 September

The common fixture blessing is complete. No expected failure is ignored and
no causal golden outside the inventory is changed. The final ordinary command
`cargo test --workspace --all-features --no-fail-fast` exits zero:
**1,545 passed, 0 failed, 0 ignored, 60 result groups**, including two empty
doctest groups. Separately, all 58 Cargo-built test executables ran once with
six workers and passed the same 1,545 tests; doctests also passed separately.
This extra orchestration neither filters tests nor changes their arguments.

An earlier ordinary rerun had **1,543 passes and two failures**: the live chat
and gateway fixtures exhausted their existing 30-second listener-start wait,
with empty child stdout/stderr. Both pass in the focused rerun and the final
complete run, without code, timeout, golden or assertion changes. This host
also showed long pre-output executable starts during the six-worker campaign.
The failed transcript is retained, not relabelled as a successful run; these
startup observations are not a protocol latency measurement.

| Gate | Observed result |
|---|---|
| Formatting | `cargo fmt --all -- --check` passes |
| Lints | strict workspace/all-target/all-feature Clippy passes |
| Documentation | workspace/no-dependency rustdoc passes |
| Portable core | `thumbv7m-none-eabi`, `thumbv8m.main-none-eabihf`, `riscv32imac-unknown-none-elf` checks pass |
| Gallery | 27 scenarios across eight programs match their goldens; all eight controlled edits expose a record difference without blessing |
| Browser | 35 isolated real-Chrome tests pass, none skipped; no application console errors |
| Other JavaScript | 16 layout, 10 chat-client and 4 lexical editor tests pass |
| Actual TextMate | 3 tokenizer tests pass with the already installed TextMate/Oniguruma tooling; no new dependency |
| Benchmark rehearsal | 20 reference graphs pass their referees; 4 runner tests and 3 canned attempts pass, 17 explicitly unattempted; no paid wave |
| Public documentation | README/quickstart exact-copy and page/front-matter tests pass; modified Markdown local file links resolve; seven UML views render |

The in-app Browser provider reports no available browser. The existing
repository harness provides local automation; it does not replace the
independent visual audit. Three fresh flagship stills at transitions 0/20/34
were visually inspected at **1352×760**. The GIF is **120 frames / 30.000 s**,
verified with ffprobe. Views 02/03/06/07 were re-rendered and visually checked
for current Contract versions, native Retry and measurement/observer ownership.

## 4. Embedded measurements — unchanged gates

The same **956-byte** Cortex-M33 Retry/full-recovery workload, three Calls and
16 four-byte frames per channel, passes with **17 records** and `SPIKE_OK`.
It uses **961,100 B text + 80,624 B rodata = 1,041,724 B**, leaving **6,852 B**
under 1 MiB. Peak heap is **130,560 B**, leaving only **512 B** under 128 KiB.
No gate is raised. These are fixed-workload measurements, not a guarantee
for maximum-size metadata tables or JSON bodies. Any further core growth
must remeasure this image first.

The freshly blessed echo image independently passes with **67 records** and
`SPIKE_OK`, its inventoried semantic hash, **112,656 B** peak heap and
**903,180 B text + 70,344 B rodata = 973,524 B**. The limits remain 128 KiB
heap and 1 MiB code, not the larger emulated allocator capacity.

## 5. Local artifacts and remaining opening gates

Both macOS optimized binaries are rebuilt with `hosted-ui,hosted-websocket`,
without HTTP/3. Each creates/simulates a starter and passes echo's three
scenarios. Intel execution is under Rosetta, not native Intel hardware.
The two archives and verified SHA-256 file are local artifacts under
`/tmp/rastok-m1-artifacts.orJti6/dist/`, not published downloads.

Both `0.1.0-alpha` crate archives are regenerated and build together after
extraction, with a command-local patch to the packaged core. Direct offline
hosted packaging without that patch correctly refuses the unpublished core.
The isolated package binary also passes the six Responses Lab scenarios.
Public metadata URLs were confirmed after these builds and are now prepared
locally; those archives predate the URL update. Publication stays disabled.

Still open, not implementation claims: Linux/remote build, independent visual
review, two unaided quickstart trials, availability of the confirmed documentation
destination, exact paid-wave model/adapters and spending authorization, and the ordered
15 October publication. No Linux target, container engine or cross-toolchain
is installed on this host. The three non-blocking protocol/contribution
qualifications remain disclosed in [release qualification](release-qualification.md).
OTLP, Budget, Saga and durable deployment storage are outside M1, not hidden
completion requirements.

Evidence in `/tmp/rastok-m1-measure.AsyHN2/`: `workspace-verified.log`,
`workspace-results.json` and `suites/`, `workspace-standard-final.log`
(the two startup failures), `gateway-startup-rerun.log`, `clippy-final.log`,
`doc.log`, `nostd-*.log`, `browser.log`, `javascript-final.log`,
`tokenizer.log`, `captures.log`, `spike.log`, `spike-echo.log`,
`spike-echo-size.log`, `package-verify-final.log`, `package-smoke.log`,
release/smoke and benchmark logs. No public push, tag, paid invocation or
external-agent review was performed.
