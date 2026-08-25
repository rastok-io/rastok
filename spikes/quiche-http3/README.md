# quiche HTTP/3 spike

> Time-boxed dependency evidence for the HTTP family planned after M1.5.
> This crate is deliberately outside the product workspace. Nothing here is a
> Connector contract, a Driver API, or a dependency commitment.

## Question

Can the low-level `quiche` API serve both sides of the future Rastok HTTP
Drivers while preserving the existing pump, correlation, and dependency
boundaries?

The spike creates a QUIC client and server over two real loopback UDP sockets,
performs a TLS handshake with certificate verification and `h3` ALPN, then
exchanges two concurrent HTTP/3 POST requests. The server answers in reverse
stream order. Success proves that the client still correlates each exact body
with its own stream while `quiche` remains independent of an async runtime.

Early data is not enabled. The certificate and private key in `certs/` are
self-signed test fixtures generated for this spike and must never be used by a
real Driver.

## Reproduce

```sh
cargo run --release --manifest-path spikes/quiche-http3/Cargo.toml
cargo test --manifest-path spikes/quiche-http3/Cargo.toml
```

The run exits non-zero on any handshake, certificate, ALPN, HTTP/3 framing,
body, FIN, or correlation failure. Its final line must start with `SPIKE_OK`.

## Pre-declared interpretation

**GO for architectural fit** requires all of the following:

- client and server use real UDP through the same pinned library without an
  async runtime;
- TLS verification and `h3` ALPN complete;
- concurrent replies remain correlated when completion order is reversed;
- all dependency types stay inside this standalone crate;
- the product workspace and its lockfile remain byte-identical.

**Not sufficient for adoption:** this self-pair does not prove public PKI,
loss recovery, cancellation, GOAWAY, trailers, large-body flow control,
HTTP/3 discovery/fallback, or interoperability with another stack.
Those are gates for the dependency decision, not conclusions inferred from a
green self-test.

Build time, dependency count and stripped release size are observations, not
pass/fail thresholds: the repository has not yet set an HTTP Driver budget.

The measured result and preliminary dependency verdict are recorded in
[`RESULTS.md`](RESULTS.md).
