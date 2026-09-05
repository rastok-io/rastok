# quiche HTTP/3 spike — results

> Observed 2026-08-26 on macOS Darwin 25.6.0, arm64, Apple M5 Pro, with
> `rustc 1.97.1`. These are dependency-spike measurements, not product
> performance claims.

## Result

**GO and selected for the M1.6 hosted HTTP/3 backend by
D-2026-08-26-1.** The comparison and product-shaped UDP/TLS gates have now
landed; H4 still owns the live HttpClient Driver.

The committed self-test passed:

```text
SPIKE_OK udp=real alpn=h3 requests=2 reverse_replies=true datagrams=20 client_packets=12/10 server_packets=10/12
```

It proves all pre-declared architectural-fit gates:

- one `quiche` version supplies both the client and server HTTP/3 APIs;
- QUIC performs a real TLS handshake, validates the pinned test certificate,
  and negotiates `h3` through ALPN;
- two simultaneous request streams remain correlated when the server replies
  in reverse order;
- the direct API needs no async runtime and cleanly separates protocol state
  from real loopback UDP I/O;
- the spike is outside the product workspace, whose `Cargo.lock` is untouched.

The upstream `http3-client` example was additionally run over real UDP against
`https://cloudflare-quic.com/` and `https://www.google.com/generate_204`; both
completed successfully. This is only a smoke observation: the upstream
example disables peer verification, Cloudflare is a same-family endpoint, and
the command is neither a deterministic test nor a CI gate.

## Dependency and build observations

| Observation | Result |
|---|---:|
| Exact direct dependency | `quiche 0.29.3` |
| Declared MSRV | Rust 1.88 |
| Direct license | BSD-2-Clause |
| Locked packages including the spike | 64 |
| Fresh release build | 21.58 s real |
| Stripped release executable | 2,114,216 bytes |
| Fresh release target tree | 251,504 KiB |
| Normal debug/test target tree | 517 MiB |
| Runtime dynamic libraries on this host | system C++, iconv, libSystem only |

The build-time number used a fresh target directory but a warm global Cargo
source cache. It included a local BoringSSL build.

The default feature selects Cloudflare's `boring` bindings (`boring 4.22.0`,
Apache-2.0; `boring-sys 4.22.0`, MIT), with CMake, bindgen and clang in the
build graph. BoringSSL is statically present in the resulting executable.
There is no Tokio dependency in the direct spike graph.

## Architectural reading

The low-level shape fits Rastok unusually well:

```text
UDP readiness / timer
        │
        ▼
quiche::Connection.recv / on_timeout
        │
        ▼
quiche::h3::Connection.poll
        │
        ▼
portable HTTP completion facts
        │
        ▼
Rastok Driver → VM/journal

Rastok request intent
        │
        ▼
quiche::h3 send_request/send_body
        │
        ▼
quiche::Connection.send
        │
        ▼
host UDP
```

Protocol progress can therefore live inside one hosted Driver pump. Neither
the synchronous VM nor `rastok-core` has to become async, and no `quiche` type
needs to cross the Connector module boundary.

The same shape also exposes the cost honestly: the application owns UDP
readiness, timers, connection IDs, address validation/retry, certificate and
trust configuration, partial-body flow control, writable-stream retries,
GOAWAY, cancellation and connection reclamation. The tiny self-pair is not a
production server skeleton.

## Adoption questions resolved or carried forward

1. **TLS authority — resolved.** H1-H2 use Rustls/Ring and H3 uses
   BoringSSL. Both receive the same exact compiled Mozilla roots and the same
   in-memory plan identity formats. Static features keep the cost optional.
2. **Real Driver pump — seam landed, protocol work remains.** The product now
   owns one bounded work/control/event pump. H4 must connect bounded UDP
   intake/output, timers, partial
   request and response bodies, cancel, timeout, GOAWAY and graceful close
   through the Rastok Driver lifecycle.
3. **Cross-stack conformance — opening proof landed.** The h3/Quinn client
   completed verified public-root H3 against Cloudflare's quiche endpoint.
   Trailers, reset and loss cases remain H4 integration work.
4. **Discovery and fallback.** Specify DNS/Alt-Svc policy and prohibit a
   protocol fallback after a request may have become externally visible.
5. **Replay safety.** Keep 0-RTT disabled until an effect/idempotency rule can
   prove which requests may be replayed.
6. **Backend substitutability — resolved for adoption.** The committed
   h3/Quinn real-UDP comparison is the documented replacement path; contract
   ids and plans contain no backend fact.
7. **WebSocket session transport.** Quiche exposes HTTP/3 Extended CONNECT but
   does not implement the RFC 6455 message machine. Prove one bounded stream
   adapter to the separately selected WebSocket codec, including settings
   negotiation, masking, fragmentation, flow control and close races, before
   claiming RFC 9220 support.

This evidence selects quiche behind Rastok's private physical seam. It does
not claim that a live H4 Driver, H3 discovery/fallback or RFC 9220 adapter
already exists.
