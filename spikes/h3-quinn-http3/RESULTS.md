# h3 + Quinn HTTP/3 comparison — results

> Observed 2026-08-26 on macOS Darwin 25.6.0, arm64, Apple M5 Pro, with
> `rustc 1.97.1`. These are dependency and integration observations, not
> product throughput claims.

## Result

**Technically viable, not selected for M1.6 H3.** The stack is the documented
substitution path if upstream maturity or the accepted dual-TLS cost changes.

The deterministic real-UDP self-pair reports:

```text
SPIKE_OK backend=h3-quinn udp=real alpn=h3 roots=public+fixture requests=2 reverse_replies=true goaway=true reclaimed=true
```

The explicitly networked interoperability smoke also passed with certificate
and hostname verification:

```text
SPIKE_OK backend=h3-quinn peer=cloudflare-quiche tls=verified alpn=h3 status=200 OK
```

The second probe is deliberately not a CI gate because DNS and the public
service are outside repository control.

## Dependency and build observations

| Observation | Result |
|---|---:|
| Direct stack | `h3 0.0.8`, `h3-quinn 0.0.10`, `quinn 0.11.11` |
| TLS/runtime | `rustls 0.23.43` with Ring, `tokio 1.53.1` |
| Direct licenses | MIT; Quinn MIT OR Apache-2.0 |
| Highest declared direct MSRV | Rust 1.85 (Quinn) |
| Locked packages including the spike | 85 |
| Fresh release build | 12.55 s real |
| Stripped release executable | 2,729,488 bytes |
| Fresh release target tree | 131,384 KiB |

The stack avoids BoringSSL/CMake/bindgen and shares Rustls with the selected
H1-H2 path. Ring still compiles its native/assembly support. Its high-level
async APIs make endpoint lifecycle concise, and the spike proves public roots,
client and server configuration, concurrent correlation, GOAWAY and endpoint
reclamation.

It loses the present adoption decision because upstream describes `h3` as
highly experimental and reserves the right to change the API; active close
and buffered-data edge cases remain under discussion. quiche's lower-level
synchronous state machine also maps more directly onto Rastok's bounded pump
and has Cloudflare production deployment evidence. This is a maturity and
integration-risk decision, not a claim that one stack is universally faster
or safer.

Neither spike proves trailers, resets under loss, discovery/fallback or
WebSocket Extended CONNECT. Those remain H4/H6 gates for whichever private
backend serves the stable contracts.
