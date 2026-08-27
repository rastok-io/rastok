# Independent WebSocket interop

This explicit hosted integration probe starts the ordinary `rastok run`
binary with one `WebSocketServer + Process` graph, then drives it through two
stacks which Rastok does not use internally:

- Node `ws` over HTTP/1.1 Upgrade;
- Python `hyper-h2` plus `wsproto` over RFC 8441 Extended CONNECT.

Run `./run.sh`. The script prints `NODE_WS_OK` and
`H2_EXTENDED_CONNECT_OK`, uses finite five-second deadlines, and leaves no
server or temporary graph behind. `npm ci` consumes the exact lockfile; `uv`
pins both Python packages on its command line. Network package installation is
therefore explicit and is not part of the hermetic Rust workspace gate.

The official Autobahn Testsuite is kept as a separate release qualification
because its maintained runner is a frozen PyPy 2 container. Its client and
server configurations live beside this probe; running them requires a
container engine and never silently substitutes a Python 3 port of the suite.
