#!/bin/sh
# Regenerate the checked-in C header for the iOS engine C ABI.
# Usage: native/ios/engine/generate-header.sh [--check]
set -eu
cd "$(dirname "$0")"
command -v cbindgen >/dev/null 2>&1 || {
  echo "cbindgen not found. Install with: cargo install cbindgen --locked" >&2
  exit 1
}
task_out=include/ExtendEngine.h
if [ "${1:-}" = "--check" ]; then
  task_tmp=$(mktemp)
  trap 'rm -f "$task_tmp"' EXIT
  cbindgen --quiet --config cbindgen.toml --crate extend-ios-engine --output "$task_tmp" .
  if ! cmp -s "$task_tmp" "$task_out"; then
    echo "$task_out is stale; run native/ios/engine/generate-header.sh" >&2
    exit 1
  fi
else
  cbindgen --quiet --config cbindgen.toml --crate extend-ios-engine --output "$task_out" .
fi
