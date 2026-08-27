#!/bin/sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cache=${RASTOK_HTTP_INTEROP_CACHE:-"$here/.cache"}
caddy_version=2.11.4
nginx_version=1.31.4
caddy_sha512=3190ae0df98b59ab4b6021556fa35adc3c526a4f3e138776b0eaec8a037cc26121cbbb1ad53453f565551b47d37d5ba4755e2c2c3652256737fe2ce9e53c8ec0
nginx_sha256=e6f20b644a17a643f059ae6467a1971fe2811587d025e071068753a1f1e3b3c3

mkdir -p "$cache/downloads" "$cache/src"

if [ -z "${CADDY:-}" ]; then
  if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "set CADDY to a pinned executable on this host" >&2
    exit 2
  fi
  archive="$cache/downloads/caddy_${caddy_version}_mac_arm64.tar.gz"
  if [ ! -f "$archive" ]; then
    curl -fsSL --max-time 60 \
      -o "$archive" \
      "https://github.com/caddyserver/caddy/releases/download/v${caddy_version}/caddy_${caddy_version}_mac_arm64.tar.gz"
  fi
  printf '%s  %s\n' "$caddy_sha512" "$archive" | shasum -a 512 -c - >/dev/null
  if [ ! -x "$cache/caddy" ]; then
    tar -xzf "$archive" -C "$cache" caddy
    chmod +x "$cache/caddy"
  fi
  CADDY="$cache/caddy"
fi

if [ -z "${NGINX:-}" ]; then
  archive="$cache/downloads/nginx-${nginx_version}.tar.gz"
  source="$cache/src/nginx-${nginx_version}"
  if [ ! -f "$archive" ]; then
    curl -fsSL --max-time 60 \
      -o "$archive" \
      "https://nginx.org/download/nginx-${nginx_version}.tar.gz"
  fi
  printf '%s  %s\n' "$nginx_sha256" "$archive" | shasum -a 256 -c - >/dev/null
  if [ ! -x "$cache/nginx/sbin/nginx" ]; then
    if [ ! -d "$source" ]; then
      tar -xzf "$archive" -C "$cache/src"
    fi
    (
      cd "$source"
      ./configure \
        --prefix="$cache/nginx" \
        --without-http_rewrite_module \
        --without-http_gzip_module >/dev/null
      make -j2 >/dev/null
      make install >/dev/null
    )
  fi
  NGINX="$cache/nginx/sbin/nginx"
fi

printf 'CADDY=%s\nNGINX=%s\n' "$CADDY" "$NGINX"
