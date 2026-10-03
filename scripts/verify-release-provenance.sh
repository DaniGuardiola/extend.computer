#!/bin/bash
# Verify the original builder even when repairing publication in a later run.
set -euo pipefail
archive=$1
directory=$(dirname "$archive")
repository=${GITHUB_REPOSITORY:-DaniGuardiola/extend.computer}
commit=$(python3 - "$directory/release-metadata.json" <<'PY'
import json, re, sys
commit = json.load(open(sys.argv[1]))['commit']
if not re.fullmatch(r'[0-9a-fA-F]{40}', commit):
    raise SystemExit('Release metadata must contain its original source commit.')
print(commit)
PY
)
gh attestation verify "$archive" \
  --bundle "$directory/provenance.sigstore.json" \
  --repo "$repository" \
  --signer-workflow "$repository/.github/workflows/release-macos.yml" \
  --source-ref refs/heads/main \
  --source-digest "$commit" \
  --deny-self-hosted-runners
