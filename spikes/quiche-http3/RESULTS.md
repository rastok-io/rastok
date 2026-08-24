# quiche HTTP/3 spike — results

> Observed 2026-08-24 on macOS Darwin 25.6.0, arm64, Apple M5 Pro, with
> `rustc 1.97.1`. These are dependency-spike measurements, not product
> performance claims.

## Result

**GO as a serious HTTP/3 backend candidate. NO-GO for immediate adoption
without a comparison against `h3` + Quinn and a product-shaped UDP/TLS
prototype.**

The committed self-test passed:

```text
SPIKE_OK alpn=h3 requests=2 reverse_replies=true datagrams=20 client_packets=12/10 server_packets=10/12
```

It proves all pre-declared architectural-fit gates:

- one `quiche` version supplies both the client and server HTTP/3 APIs;
- QUIC performs a real TLS handshake, validates the pinned test certificate,
  and negotiates `h3` through ALPN;
- two simultaneous request streams remain correlated when the server replies
  in reverse order;
- the direct API needs no async runtime and cleanly separates protocol state
  from datagram I/O;
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
| Empty-target release build | 33.53 s real |
| Stripped release executable | 2,113,312 bytes |
| Normal release target tree | 247 MiB |
| Normal debug/test target tree | 517 MiB |
| Runtime dynamic libraries on this host | system C++, iconv, libSystem only |

The build-time number used an empty spike target but a warm global Cargo cache
for the `quiche` crate source itself. It included index/download work for the
remaining dependencies and a local BoringSSL build.

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
needs to cross the family module boundary.

The same shape also exposes the cost honestly: the application owns UDP
readiness, timers, connection IDs, address validation/retry, certificate and
trust configuration, partial-body flow control, writable-stream retries,
GOAWAY, cancellation and connection reclamation. The tiny self-pair is not a
production server skeleton.

## Adoption questions still open

1. **One TLS authority.** `quiche` defaults to BoringSSL, whereas the likely
   HTTP/1.1–2 stack points toward Hyper + rustls. Shipping both TLS stacks in
   the standard build needs an explicit cost/security decision.
2. **Real Driver pump.** Prove bounded UDP intake/output, timers, partial
   request and response bodies, cancel, timeout, GOAWAY and graceful close
   through the Rastok Driver lifecycle.
3. **Cross-stack conformance.** Run client and server separately against at
   least one independent implementation with certificate verification,
   trailers, reset and loss cases.
4. **Discovery and fallback.** Specify DNS/Alt-Svc policy and prohibit a
   protocol fallback after a request may have become externally visible.
5. **Replay safety.** Keep 0-RTT disabled until an effect/idempotency rule can
   prove which requests may be replayed.
6. **Backend substitutability.** Compare the same product-shaped seam with
   `h3` + Quinn. The semantic Connector id and plan must remain identical.

This evidence supports including HTTP/3 in Rastok's hosted architecture from
the beginning. It does not yet choose `quiche` as the implementation.
