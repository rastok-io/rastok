# Release qualification

> This is the single inventory of evidence deliberately kept outside the
> hermetic Rust workspace gate. A milestone report may link here, but may not
> silently omit an open row or turn it into a product-readiness claim.

## Executed on the reference host

| Surface | Independent implementation | Evidence | Last result |
|---|---|---|---|
| HttpClient HTTP/1.1 | Caddy 2.11.4 | [`spikes/http-interop`](../../spikes/http-interop/) | `CADDY_HTTP1_CLIENT_OK` — rerun 2026-08-28 |
| HttpClient HTTP/1.1 | NGINX 1.31.4 | [`spikes/http-interop`](../../spikes/http-interop/) | `NGINX_HTTP1_CLIENT_OK` — rerun 2026-08-28 |
| HttpServer HTTP/1.1 | curl 8.7.1 | [`spikes/http-interop`](../../spikes/http-interop/) | `CURL_HTTP1_SERVER_OK` — rerun 2026-08-28 |
| HttpServer HTTP/2 | nghttp2 1.70.0 | [`spikes/http-interop`](../../spikes/http-interop/) | `NGHTTP2_SERVER_OK` — rerun 2026-08-28 |
| WebSocketServer HTTP/1.1 Upgrade | Node `ws` 8.21.3 | [`spikes/websocket-interop`](../../spikes/websocket-interop/) | `NODE_WS_OK` — rerun 2026-08-28 |
| WebSocketServer HTTP/2 Extended CONNECT | hyper-h2 4.3.0 + wsproto 1.3.2 | [`spikes/websocket-interop`](../../spikes/websocket-interop/) | `H2_EXTENDED_CONNECT_OK` — rerun 2026-08-28 |

These are blocking loopback probes with finite deadlines. Package retrieval is
explicit, pinned and separate from `cargo test`; it is not a hidden network
dependency of normal development or CI.

## Open before a public release

| Qualification | Required environment | Closure evidence |
|---|---|---|
| ngtcp2 client against live Rastok HTTP/3 server | current C++23 compiler, nghttp3, event loop and explicit supported TLS backend | pinned command, versions and successful terminal transcript |
| Official Autobahn RFC 6455 client and server roles | container-capable host for the maintained frozen PyPy 2 runner | client/server HTML reports linked from the release record |
| Marginal Connector implementation benchmark | one new representative Connector after M1.6, measured from empty family module to conformance | repeatable timing and scope record in the release report |

Public Internet probes remain scheduled canaries, never replacements for these
blocking local qualifications. The gallery, website and demonstration are M1
deliverables tracked by the roadmap rather than protocol-conformance evidence.
