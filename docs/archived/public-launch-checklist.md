> Archived proposal or historical report. Claims and instructions describe the document’s original milestone, not the current app. See the [current documentation](../README.md).

# Clean public launch checklist

This is the pre-public reset plan. Nothing below authorizes deleting data or
rewriting history merely because it appears on this checklist. Complete the
reset before the first supported public installation; after launch, preserve
update continuity and support the published data/protocol formats.

## Preserve what survives the reset

- [ ] Confirm the six release recovery files and instructions are present in
  `extend.computer-release-credentials` in Proton Drive My Files, and confirm
  the desktop client has finished syncing. A local copy alone does not prove sync.
- [ ] Verify recovery of the Apple signing certificate/private key, notarization
  API key, and Sparkle signing key from the backup. Keep these through the reset.
- [ ] Choose the permanent bundle/helper identifiers, update-feed URL, first
  release version, and supported macOS versions. Keep published identifiers,
  keys, and feed URLs stable afterward.
- [ ] Confirm all existing installations are disposable development/test builds
  before resetting versions, schemas, protocol baselines, or build counters.

## Remove pre-launch migration baggage

- [ ] Inventory legacy-data readers, schema upgrade paths, renamed-setting
  fallbacks, old account/session formats, development protocol adapters, and
  their tests. Remove paths needed only to preserve disposable pre-launch state.
- [ ] Review the legacy device-name and local-placeholder cleanup in
  `desktop/src-tauri/src/runtime.rs` and related runtime tests as initial candidates.
- [ ] Consolidate hosted and standalone database schema history into a complete
  fresh-install baseline where appropriate. Replace historical upgrade chains
  and their bookkeeping with initialization that works on an empty database.
- [ ] Keep required current-schema initialization and migrations needed by any
  retained environment. Plan explicit resets of test databases/accounts rather
  than silently assuming deployed data matches the new baseline.
- [ ] Keep protocol/capability negotiation, helper API checks, and clear
  update-required errors. These protect future releases even with a clean launch.
- [ ] Test empty-profile startup, account registration/login/2FA, pairing,
  permissions, and sharing after cleanup. Confirm no removed migration is still
  required to create or operate a fresh installation.

## Reset repository and disposable release state

- [ ] Save a recoverable private snapshot of the complete working tree and
  current history, including uncommitted work, before rewriting anything.
- [ ] Review source and history for credentials, personal data, local databases,
  generated installers, and unintended files. Ensure the new public snapshot
  contains no signing material; revoke/rotate any credential found exposed.
- [ ] Squash the entire repository into one intentional initial commit containing
  the final source, docs, dependency locks, CI workflows, and launch metadata.
- [ ] Remove obsolete test branches/PRs/tags, test GitHub releases and installers,
  old workflow artifacts, and disposable provenance records. Account for refs
  that can still expose pre-reset commits; rewriting main alone is not a purge.
- [ ] Delete the private `v0.1.0` test release and its tag before the clean launch,
  including both versioned and fixed-name DMGs, checksums, metadata, and test
  update-feed entries. Delete any subsequent test releases/tags too. It is a
  local working-tree test build, not a provenance-attested public release.
- [ ] Delete the subsequent CI test releases/tags (starting with `v0.2.0`) and
  their assets/feed entries before choosing the clean public launch baseline.
- [ ] Reset the `updates` branch and stable/alpha/beta/canary feeds to the chosen
  launch baseline. Remove test entries and re-sign every changed feed.
- [ ] Reset the changelog, consumed Changesets prerelease notes/state, and version
  metadata together. Keep real launch notes; omit prototype/test release history.
- [ ] Check package, Rust, Tauri, lockfile, feed, and installer versions agree.
  Choose a fresh build baseline now; keep build numbers increasing after launch.
- [ ] Ensure temporary branch protection changes made for the history rewrite
  are restored afterward.

## Prepare GitHub and public source

- [x] Select a project license: MIT, recorded in the root `LICENSE`.
- [ ] Confirm vendored license notices and credits.
- [ ] Update README, setup/build instructions, supported features, security
  reporting contact, and self-hosting instructions for the public baseline.
- [ ] Upgrade the pinned macOS runner/Xcode before its hosted image is retired.
- [ ] Push the final workflows and run PR checks successfully. Confirm clean
  installation from the root lockfile and the pinned Node/Rust versions.
- [ ] Install the official Changesets bot for this repository only and verify its
  PR comment detects root `.changeset` files. Verify the release-PR workflow
  creates/updates a PR without approving or merging it.
- [ ] Verify `desktop-release` secrets, squash-only merging, merged-branch cleanup,
  read-only default Actions permissions, and the intended signing-job access.
- [ ] Make the repository public only after the source/reset review is complete.
- [ ] Enable main protection with `bash scripts/protect-main.sh` as soon as the
  plan/visibility supports it. Verify required checks, PRs, conversation
  resolution, linear history, and blocked force pushes/deletion.
- [ ] Configure GitHub Pages from `updates` / root. Verify the permanent feed URLs
  and release downloads work anonymously, without GitHub tokens in the app.
- [ ] Verify the landing page's download opens the latest stable universal DMG,
  and alpha/beta publication does not change that stable download.

## Verify the first public release pipeline

- [ ] Verify password recovery and email verification end to end on the public
  account service: configured sender/credentials, delivery, reset link, expiry,
  one-use tokens, and preserved two-factor authentication. The local UI review
  currently reports “Email recovery is not available yet” because email is
  disabled in the development environment; confirm intended preview behavior
  and ensure release users can recover their accounts.

- [ ] Build a new release in public GitHub Actions. Private/local test binaries
  have no public CI provenance and must not be relabeled as attested builds.
- [ ] Verify Apple notarization is **Accepted**, app and DMG tickets are stapled,
  and a downloaded installer passes Gatekeeper checks. Authentication/signing
  success alone is not notarization acceptance.
- [ ] Verify provenance is generated for the final signed/stapled DMG, published
  as `provenance.sigstore.json`, and required before public publication.
- [ ] Run `gh attestation verify` on the downloaded DMG, enforcing this repo,
  release workflow, main ref, and expected source commit. Confirm modified
  artifacts fail verification.
- [ ] Verify publication repair uses the original attestation/source commit and
  cannot publish an artifact with missing or invalid provenance.
- [ ] Verify SHA-256 checksums, Sparkle archive signatures, signed feeds, release
  notes, and the bundled offline changelog all match the launch artifacts.
- [ ] Record that provenance is enabled but byte-for-byte reproducibility remains
  future work; macOS/Xcode/SDK inputs and signing timestamps are not fully pinned.

## Exercise installation and updates on real Macs

- [ ] Install from the downloaded DMG on Apple Silicon and Intel, including a
  fresh machine/profile with no development tooling or cached permissions.
- [ ] Test two signed/notarized builds through a staging feed and then the public
  feed: manual checks, automatic checks/downloads, replacement, and relaunch.
- [ ] Test alpha and beta releases and the Settings → Updates channel selector.
  Verify stable excludes prereleases and returning to stable waits for a newer
  build rather than downgrading. Verify Changesets preview graduation works.
- [ ] Update while sharing: wait for the active session to end, reserve the idle
  runtime, and clean up input/listeners before installation. Confirm no new
  sharing session can start once installation is committed.
- [ ] Verify pairing identities, saved account credentials, permissions, and
  Wi-Fi helper approval survive an update between supported public formats.
- [ ] Test tampered feed/archive rejection, unavailable hosting, canceled
  downloads, and retry after partial publication.
- [ ] Test mixed compatible device versions and incompatible protocol ranges or
  missing capabilities. Confirm clear errors, no input injection, and no lost
  pairings.
- [ ] Complete a final fresh-install smoke test against the reset hosted account
  service, then publish the launch downloads and announcement.

Implementation and verification commands: [macOS releases](../operations/releases.md).
