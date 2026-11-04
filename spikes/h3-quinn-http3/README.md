# h3 + Quinn HTTP/3 comparison

> Product-shaped dependency comparison for M1.6 H3. This crate is outside
> the product workspace and does not select a backend by itself.

The spike binds a Quinn server and client to real loopback UDP sockets. Its
Rustls client store contains the public WebPKI roots plus the pinned local test
CA, early data is disabled, and `h3` serves two concurrent requests in reverse
completion order before graceful GOAWAY and endpoint reclamation.

```sh
cargo run --release --manifest-path spikes/h3-quinn-http3/Cargo.toml
cargo test --manifest-path spikes/h3-quinn-http3/Cargo.toml
```

Success ends with `SPIKE_OK`. The dedicated CA and leaf fixtures in `certs/`
are self-signed test-only material for `localhost`, generated for this spike,
and must never be used by a real Driver.

An explicitly networked, non-CI smoke probe exercises the h3/Quinn client
against Cloudflare's quiche endpoint with public-root and hostname
verification:

```sh
cargo run --release --manifest-path spikes/h3-quinn-http3/Cargo.toml -- --public
```

It is adoption evidence, not a deterministic repository gate: DNS and the
public service are outside Rastok's control.

The result is comparison evidence, not a live Rastok Driver. In particular it
does not erase upstream's experimental API status or independently prove an
HTTP/3 WebSocket Extended CONNECT adapter.
