#!/bin/sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rastok-websocket-interop.XXXXXX")
port=${RASTOK_WS_PORT:-19081}
server_pid=

finish() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$tmp"
}
trap finish EXIT INT TERM

sed "s/__PORT__/$port/g" "$here/gateway.rkg.in" > "$tmp/gateway.rkg"
cargo build --quiet --manifest-path "$repo/Cargo.toml" -p rastok --features hosted-websocket
"$repo/target/debug/rastok" run "$tmp/gateway.rkg" >"$tmp/server.out" 2>"$tmp/server.err" &
server_pid=$!

python3 - "$port" <<'PY'
import socket
import sys
import time

port = int(sys.argv[1])
deadline = time.monotonic() + 5
while True:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
            break
    except OSError:
        if time.monotonic() >= deadline:
            raise
        time.sleep(0.02)
PY

(cd "$here" && npm ci --silent)
node "$here/node-client.mjs" "ws://127.0.0.1:$port/echo" "node-independent"
uv run --quiet --with 'h2==4.3.0' --with 'wsproto==1.3.2' \
  "$here/h2-client.py" 127.0.0.1 "$port" /echo h2-independent
