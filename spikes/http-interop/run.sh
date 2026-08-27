#!/bin/sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rastok-http-interop.XXXXXX")
caddy_port=${RASTOK_CADDY_PORT:-19082}
nginx_port=${RASTOK_NGINX_PORT:-19083}
rastok_port=${RASTOK_HTTP_PORT:-19084}
caddy_pid=
nginx_pid=
rastok_pid=

finish() {
  for pid in "$rastok_pid" "$nginx_pid" "$caddy_pid"; do
    if [ -n "$pid" ]; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  rm -rf "$tmp"
}
trap finish EXIT INT TERM

wait_for_tcp() {
  python3 - "$1" <<'PY'
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
}

tools=$("$here/fetch-tools.sh")
CADDY=$(printf '%s\n' "$tools" | sed -n 's/^CADDY=//p')
NGINX=$(printf '%s\n' "$tools" | sed -n 's/^NGINX=//p')
test -x "$CADDY"
test -x "$NGINX"
CURL=${CURL:-curl}
NGHTTP=${NGHTTP:-nghttp}

mkdir -p "$tmp/caddy-root" "$tmp/nginx-root"
printf 'caddy-independent\n' > "$tmp/caddy-root/body"
printf 'nginx-independent\n' > "$tmp/nginx-root/body"

cat > "$tmp/Caddyfile" <<EOF
{
  admin off
  auto_https off
}
http://127.0.0.1:$caddy_port {
  root * $tmp/caddy-root
  file_server
}
EOF
"$CADDY" run --config "$tmp/Caddyfile" --adapter caddyfile \
  >"$tmp/caddy.out" 2>"$tmp/caddy.err" &
caddy_pid=$!

cat > "$tmp/nginx.conf" <<EOF
pid $tmp/nginx.pid;
error_log $tmp/nginx.err;
events { worker_connections 32; }
http {
  access_log $tmp/nginx.access;
  server {
    listen 127.0.0.1:$nginx_port;
    root $tmp/nginx-root;
  }
}
EOF
"$NGINX" -c "$tmp/nginx.conf" -g 'daemon off;' \
  >"$tmp/nginx.out" 2>"$tmp/nginx.stderr" &
nginx_pid=$!

wait_for_tcp "$caddy_port"
wait_for_tcp "$nginx_port"

cargo build --quiet --manifest-path "$repo/Cargo.toml" -p rastok --features hosted-http12

sed "s|__URL__|http://127.0.0.1:$caddy_port/body|g" \
  "$here/client.rkg.in" > "$tmp/caddy.rkg"
if ! printf 'request\n' | "$repo/target/debug/rastok" run "$tmp/caddy.rkg" \
  > "$tmp/caddy-client.out" 2> "$tmp/caddy-client.err"; then
  cat "$tmp/caddy-client.err" >&2
  exit 1
fi
if ! rg -q 'caddy-independent' "$tmp/caddy-client.out"; then
  cat "$tmp/caddy-client.out" >&2
  exit 1
fi
printf 'CADDY_HTTP1_CLIENT_OK\n'

sed "s|__URL__|http://127.0.0.1:$nginx_port/body|g" \
  "$here/client.rkg.in" > "$tmp/nginx.rkg"
if ! printf 'request\n' | "$repo/target/debug/rastok" run "$tmp/nginx.rkg" \
  > "$tmp/nginx-client.out" 2> "$tmp/nginx-client.err"; then
  cat "$tmp/nginx-client.err" >&2
  exit 1
fi
if ! rg -q 'nginx-independent' "$tmp/nginx-client.out"; then
  cat "$tmp/nginx-client.out" >&2
  exit 1
fi
printf 'NGINX_HTTP1_CLIENT_OK\n'

sed "s/__PORT__/$rastok_port/g" "$here/server.rkg.in" > "$tmp/server.rkg"
"$repo/target/debug/rastok" run "$tmp/server.rkg" \
  >"$tmp/server.out" 2>"$tmp/server.err" &
rastok_pid=$!
wait_for_tcp "$rastok_port"

status=$("$CURL" --silent --show-error --max-time 5 --http1.1 \
  --output "$tmp/curl.body" --write-out '%{http_code}' \
  --data 'curl-independent' "http://127.0.0.1:$rastok_port/")
test "$status" = 202
printf 'CURL_HTTP1_SERVER_OK\n'

printf 'nghttp-independent' > "$tmp/nghttp.body"
"$NGHTTP" --verbose --timeout=5 --data "$tmp/nghttp.body" \
  "http://127.0.0.1:$rastok_port/" \
  >"$tmp/nghttp.out" 2>&1
rg -q ':status: 202' "$tmp/nghttp.out"
printf 'NGHTTP2_SERVER_OK\n'
