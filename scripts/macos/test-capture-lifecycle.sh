#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
output_dir=${1:-"$PWD/target/capture-lifecycle-check"}
mkdir -p "$output_dir"
xcrun swiftc -O -parse-as-library -module-cache-path "$output_dir/module-cache" \
    native/macos/display/CaptureLifecycle.swift scripts/macos/tests/capture-lifecycle.swift \
    -o "$output_dir/check"
"$output_dir/check"
python3 - "$output_dir/check" <<'PY'
import json
import subprocess
import sys

binary = sys.argv[1]
results = {}
for mode in ('pipe-baseline', 'pipe-viewer', 'pipe-source'):
    result = subprocess.run([binary, mode], capture_output=True, text=True, timeout=10)
    results[mode] = {'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}
assert results['pipe-viewer']['exit'] == results['pipe-baseline']['exit'], results
assert results['pipe-source']['exit'] == 0, results
assert 'EPIPE reached sender shutdown' in results['pipe-source']['stdout'], results
print(json.dumps({'real_closed_pipe_checks': results}, indent=2))
PY
