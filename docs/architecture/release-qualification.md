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

## Open before a public release

| Qualification | Required environment | Closure evidence |
|---|---|---|
| ngtcp2 client against live Rastok HTTP/3 server | current C++23 compiler, nghttp3, event loop and explicit supported TLS backend | pinned command, versions and successful terminal transcript |
| Official Autobahn RFC 6455 client and server roles | container-capable host for the maintained frozen PyPy 2 runner | client/server HTML reports linked from the release record |
| Marginal Connector implementation benchmark | one new representative Connector after M1.6, measured from empty contract-specific module to conformance | repeatable timing and scope record in the release report |
| M1.9 hosted UI visual and interaction pass | a working real-browser controller on the reference host | initial-pause/control/reconnect/reset/completed-state transcript; read-only simulation back/forward and reset-session isolation; buffered Play at 1/4 events per second; mailbox/instance inspection, zoom/pan and splitter pass; keyboard pass; same-viewport 1352×760 captures at B0, B1, B8 and the completed boundary plus a narrow screenshot linked from the M1.9 report |

R9 attempted the browser pass on 2026-09-07. The installed controller failed
before connection: its worker referenced an absent older browser-service module.
No visual/keyboard pass or screenshots are claimed. HTTP/controller/timeline,
redaction and SVG geometry tests remain separate executable evidence.

Public Internet probes remain scheduled canaries, never replacements for these
blocking local qualifications. The gallery, website and demonstration are M1
deliverables tracked by the roadmap rather than protocol-conformance evidence.

## Embedded code-size watch before Saga

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
