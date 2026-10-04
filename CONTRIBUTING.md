# Contributing to extend.computer

## Development setup

Desktop development currently requires macOS, Xcode with Swift 6 support, Rust
via rustup, and Node/npm. Use the versions in `rust-toolchain.toml` and
`.node-version`; native CI jobs select Xcode 16.2. Python 3.11 or newer is required
for the release scripts.

Run commands from the repository root unless stated otherwise:

```sh
npm ci
rustup show
npm run desktop:dev --prefix desktop
```

The root npm workspace includes the desktop app. The website has its own
dependencies and deployment process; see [web/README.md](web/README.md).
See the [desktop guide](docs/archived/desktop-preview.md) for local signing, permission
setup, and development identities.

## Pull requests and validation

Keep changes focused and describe the resulting behavior and relevant
verification. Add a Changeset for user-visible desktop changes:

```sh
npm run changeset
```

Select `extend-computer-desktop`, choose patch/minor/major, and write notes for
users. Commit the generated root `.changeset/*.md` file with the change.
Documentation and test-only changes can omit a Changeset. Missing notes are
advisory in CI; malformed configuration and other validation errors still fail.

The `macOS checks` job validates release metadata, builds the native resources
and frontend, and runs core, desktop, and release-tool tests. Local equivalents:

```sh
sh scripts/macos/build-cursor-helper.sh
npm run native:permissions --prefix desktop
npm run build --prefix desktop
cargo test --locked
cargo test --manifest-path desktop/src-tauri/Cargo.toml --locked
python3 scripts/test-release.py
npm run release:check
```

Run the Rust suites sequentially to avoid competing local port allocations.
Some tests require local networking; the isolated account-relay integration
test is ignored by default and documents its separate prerequisites.

## Release a macOS version

Use the normal Changesets and GitHub Actions process:

1. Add a Changeset with the implementation and merge its PR into `main` after
   checks pass. Do not manually bump app versions for an ordinary release.
2. The **Prepare desktop release** workflow creates or updates the
   `codex/desktop-release` PR with version bumps and generated changelog notes.
   It synchronizes the desktop package, Rust manifests and lockfiles, Tauri
   configuration, and root npm lockfile. Website and standalone account-server
   versions remain separate.
3. Review that release PR. If GitHub marks its Actions run as requiring
   approval, approve the reviewed run in GitHub. The workflow does not approve
   or merge its own PR. Wait for `macOS checks` to pass, then squash-merge.
4. Merging starts **Release macOS app** automatically. CI builds one universal
   Apple Silicon/Intel app, signs it with Developer ID, notarizes and staples
   the app and DMG, and signs the installer and channel feed with Sparkle.
5. Confirm the release workflow succeeds. It publishes GitHub release assets
   first, then signed feeds on the `updates` branch. Public releases also
   require build provenance and verification before publication.
6. Check the uploaded installer, notarization results, checksums, release
   metadata, and feed. Exercise installation and updating as appropriate for
   the change; a successful upload alone is not proof of a working update.

Release signing uses the `desktop-release` GitHub environment. Contributors do
not need production credentials. Never commit certificates, private keys,
passwords, local databases, or generated installers. Maintainer setup and
verification commands are in [macOS releases](docs/operations/releases.md).

Release notes come from the generated desktop changelog and are bundled into
the app's offline “What's new” view. The stable landing-page download points to
`releases/latest/download/extend.computer-macos-universal.dmg`; CI publishes an
identical fixed-name copy of each stable installer. No website change is needed
for each desktop release.

## Alpha, beta, and canary channels

Enter prerelease mode before preparing a preview release:

```sh
npm run changeset -- pre enter beta
```

Use `alpha` or `canary` instead of `beta` for those channels. Commit the generated
prerelease state, add Changesets for changes, and follow the same release-PR
process. CI infers the channel from the version suffix. Canary uses explicit
Changesets prereleases; it is not automatically published on every commit.

To graduate the preview series to stable:

```sh
npm run changeset -- pre exit
```

Commit the exit state and merge the generated graduation release PR after
checks pass. Preview releases use separate feeds and do not replace GitHub's
latest stable download. Users choose channels in Settings → Updates; switching
back to stable waits for a newer stable build rather than downgrading.

## Failed publication and release continuity

If assets uploaded successfully but feed publication failed, fix the workflow
and run **Release macOS app** from `main` with the matching channel. Repair runs
reuse published installer bytes and their original build number; public repair
runs also verify the original provenance. The workflow skips a release whose
feed is already published. To retry unchanged workflow code, rerun the failed
workflow in GitHub Actions.

Fix a defective published app with a new version, not by replacing an existing
installer. Preserve the app identifiers, feed URLs, Sparkle key, and increasing
build numbers after public launch. App versions and device protocol versions
are independent: changes that affect communication must retain compatibility
negotiation or report clearly that an update is required.

While the repository is private, downloads require GitHub access and the
configured public update-feed URLs are unavailable until GitHub Pages is
enabled. Private test releases on the current plan skip provenance. Making the
repository public enables mandatory provenance in CI; it still needs a live
public release verification.

For the planned pre-public history and test-release reset, follow the
[public launch checklist](docs/archived/public-launch-checklist.md). Preserve signing
credentials and explicitly confirm which installations and release state are
disposable before resetting anything.
