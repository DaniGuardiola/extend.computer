#!/bin/sh
set -eu
base=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
python_bin=${BENCHMARK_PYTHON:-"$base/.venv/bin/python3"}
if [ ! -x "$python_bin" ]; then
  echo "Run ./benchmarks/display/run setup first." >&2
  exit 1
fi
exec "$python_bin" "$base/camera_run.py" "$@"
