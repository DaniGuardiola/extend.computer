#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $(python3 scripts/release.py pending) != true ]]; then
  exit 0
fi
branch=codex/desktop-release
git config user.name 'github-actions[bot]'
git config user.email '41898282+github-actions[bot]@users.noreply.github.com'
git switch -C "$branch"
npm run release:version --prefix desktop
version=$(node -p "require('./desktop/package.json').version")
git add .changeset desktop/CHANGELOG.md desktop/package.json package-lock.json \
  Cargo.toml Cargo.lock desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock desktop/src-tauri/tauri.conf.json
git commit -m "Prepare extend.computer $version release"
git push --force-with-lease origin "HEAD:$branch"
body=$(mktemp)
trap 'rm -f "$body"' EXIT
python3 scripts/release.py notes > "$body"
number=$(gh pr list --head "$branch" --base main --state open --json number --jq '.[0].number // empty')
if [[ -n $number ]]; then
  gh pr edit "$number" --title "Release extend.computer $version" --body-file "$body"
else
  gh pr create --head "$branch" --base main --title "Release extend.computer $version" --body-file "$body"
fi
