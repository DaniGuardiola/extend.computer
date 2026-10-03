#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
tag="v$RELEASE_VERSION"
if [[ ${REQUIRE_PROVENANCE:-false} == true ]]; then
  bash scripts/verify-release-provenance.sh "release-output/extend.computer-$RELEASE_VERSION-universal.dmg"
fi
# A stable asset name lets the website use GitHub's latest/download redirect.
# Copy only after provenance verification; these are identical installer bytes.
download_alias="release-output/extend.computer-macos-universal.dmg"
cp "release-output/extend.computer-$RELEASE_VERSION-universal.dmg" "$download_alias"
if ! gh release view "$tag" --repo "$RELEASE_REPOSITORY" >/dev/null 2>&1; then
  target=main
  [[ $RELEASE_REPOSITORY != "$GITHUB_REPOSITORY" ]] || target="$GITHUB_SHA"
  args=("$tag" --repo "$RELEASE_REPOSITORY" "release-output/extend.computer-$RELEASE_VERSION-universal.dmg" release-output/SHA256SUMS release-output/release-metadata.json --title "extend.computer $RELEASE_VERSION" --notes-file release-output/release-notes.md --target "$target")
  args+=("$download_alias")
  if [[ -f release-output/provenance.sigstore.json ]]; then args+=(release-output/provenance.sigstore.json); fi
  if [[ $RELEASE_CHANNEL != stable ]]; then args+=(--prerelease --latest=false); else args+=(--latest); fi
  gh release create "${args[@]}"
fi
# Feeds are published only after their referenced assets exist. This branch is
# the GitHub Pages source; configure Pages for updates / root in repo settings.
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
cp -R release-output/updates "$temporary/updates"
git -C "$temporary" init -b updates
git -C "$temporary" config user.name github-actions\[bot\]
git -C "$temporary" config user.email '41898282+github-actions[bot]@users.noreply.github.com'
git -C "$temporary" remote add origin "https://github.com/$RELEASE_REPOSITORY.git"
if git -C "$temporary" ls-remote --exit-code --heads origin updates >/dev/null; then
  git -C "$temporary" fetch origin updates
  git -C "$temporary" reset --mixed FETCH_HEAD
fi
touch "$temporary/.nojekyll"
git -C "$temporary" add updates .nojekyll
git -C "$temporary" commit -m "Publish $RELEASE_CHANNEL updates for $RELEASE_VERSION"
# gh's credential helper reads the scoped distribution token from GH_TOKEN.
# Do not write credentials into the repository or remote URL.
git -C "$temporary" -c credential.helper= -c 'credential.helper=!gh auth git-credential' push origin HEAD:updates
# GITHUB_TOKEN pushes do not trigger the automatic Pages build. Request one
# explicitly once Pages is enabled; private repositories may enable it later.
if gh api "repos/$RELEASE_REPOSITORY/pages" --silent >/dev/null 2>&1; then
  gh api --method POST "repos/$RELEASE_REPOSITORY/pages/builds" --silent
else
  echo '::warning::Update feeds are saved on the updates branch. Enable GitHub Pages from updates / root when public downloads are needed.'
fi
