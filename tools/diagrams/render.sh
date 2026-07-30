#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
diagram_directory="$repository_root/docs/architecture"
plantuml_executable="${PLANTUML:-plantuml}"

if ! command -v "$plantuml_executable" >/dev/null 2>&1; then
  echo "PlantUML not found. Install it or set PLANTUML to its executable." >&2
  exit 1
fi

"$plantuml_executable" \
  -charset UTF-8 \
  -tsvg \
  "$diagram_directory"/*.puml
