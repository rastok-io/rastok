# Independent HTTP interop

This explicit hosted probe crosses each Rastok HTTP/1.1–2 boundary with an
implementation which Rastok does not use internally:

- Rastok `HttpClient` reads fixed responses from Caddy 2.11.4 and NGINX
  1.31.4;
- curl sends an HTTP/1.1 request to Rastok `HttpServer`;
- nghttp2 sends an HTTP/2 prior-knowledge request to the same Rastok
  contract.

Run `./run.sh`. It prints four `*_OK` lines, imposes finite five-second
protocol deadlines, and removes every process and temporary graph it starts.
`fetch-tools.sh` downloads exact upstream archives into the ignored `.cache`
directory and verifies their published digest before use. Callers may instead
set `CADDY`, `NGINX`, `CURL`, or `NGHTTP` to pinned local executables.

Automatic tool bootstrap currently targets Apple Silicon, the reference host
on which this evidence was recorded. On another host, supply `CADDY` and
`NGINX`; the protocol probe itself is otherwise ordinary POSIX shell.

The ngtcp2 HTTP/3 example client is deliberately kept in the project-wide
release-qualification list. Its current build requires a separate C++23,
nghttp3, event-loop, and TLS-backend toolchain; silently replacing that named
client with Rastok's own quiche stack would defeat the purpose of this probe.
