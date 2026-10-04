# macOS releases and compatibility

[← CLI operation](cli.md) · [Contents](../README.md) · [Documentation →](../README.md)

> [!NOTE]
> macOS only. Other platform adapters are **not supported yet**.

The macOS app embeds Sparkle 2.10.0 (MIT). GitHub Actions builds releases;
GitHub Releases hosts downloads; a static, signed Sparkle feed advertises them.
No Sparkle service or account is involved. System profiling is disabled, and
release notes contain no remote images or JavaScript. Update hosting still sees
ordinary HTTP requests, including client IP addresses.

## One-time setup

1. Enroll in the [Apple Developer Program](https://developer.apple.com/programs/enroll/).
2. Create a **Developer ID Application** certificate with its matching private
   key. Export both from Keychain Access as a password-protected `.p12`.
   Do not use an Apple Development or Mac App Store distribution certificate.
3. Create an App Store Connect **team API key** for notarization. Save its `.p8`
   file, key ID, and issuer ID. Individual keys are not supported by this pipeline.
4. In GitHub, create environment `desktop-release`. Add these environment secrets:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded `.p12` file |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting that export |
| `APPLE_SIGNING_IDENTITY` | Full `Developer ID Application: … (TEAMID)` identity |
| `APPLE_TEAM_ID` | Apple signing team ID |
| `APPLE_API_KEY_ID` | App Store Connect team API key ID |
| `APPLE_API_ISSUER` | Team API key issuer ID |
| `APPLE_API_KEY` | Complete `.p8` contents |
| `SPARKLE_PRIVATE_KEY` | Exported Sparkle Ed25519 private key |
| `SPARKLE_PUBLIC_KEY` | Matching public key |

The public key is not confidential. Keeping it with the environment configuration
ensures the app and update feed use the same key. Never commit private signing
material or paste it into chat.

Generate the Sparkle key on a trusted Mac:

```sh
bash scripts/build-updater.sh
native/macos/Updater/vendor/bin/generate_keys --account computer.extend.updates
native/macos/Updater/vendor/bin/generate_keys --account computer.extend.updates -p
```

The private key stays in the login Keychain. To transfer it to GitHub Secrets,
export with `generate_keys --account computer.extend.updates -x /path/to/private-file`,
protect that file, upload with `gh secret set SPARKLE_PRIVATE_KEY --env desktop-release < /path/to/private-file`,
then remove the transfer file. Keep a secure backup. All channels share this key.

5. Enable GitHub Actions to create pull requests in repository settings.
   GitHub uses one switch for creating and approving pull requests. The release
   workflow only creates/updates release PRs; it never approves or merges them.
   Keep default workflow permissions read-only; release jobs request their own
   explicit write permissions. Bot-created PR workflows may need a maintainer
   to approve the first run when using `GITHUB_TOKEN`.
6. Configure GitHub Pages to deploy from branch **`updates`**, root directory.
   The first release creates this branch. Installers and feeds use the existing
   `DaniGuardiola/extend.computer` repository. While it remains private, visitors
   and updater clients may receive GitHub 404 responses; this is expected until
   the repository becomes public. No GitHub credentials are embedded in the app.
   The publishing script explicitly requests a Pages build after saving feeds,
   because pushes made with the Actions token do not trigger Pages automatically.

Default feed base is
`https://DaniGuardiola.github.io/extend.computer/updates/macos`.
Set repository variable `UPDATE_FEED_BASE` to another HTTPS base before the first
release if desired. For example, serve the same static feed files at
`https://extend.computer/updates/macos`. Keep this URL reachable for installed apps.

CI pins Node 24.19.0 (`.node-version`) and Rust 1.98.1 (`rust-toolchain.toml`
and workflow inputs). Other supported local Node versions are 22.11+ within
Node 22, Node 24, or Node 26+; npm 10.9+. Update the pins deliberately together.
Install dependencies with `npm ci` at the repository root. The root workspace
contains only `desktop`, and `.changeset` at the root is shared by the CLI and
official PR bot. `package-lock.json` at the root is the dependency lockfile.
For universal local builds, use one current rustup toolchain with both
`aarch64-apple-darwin` and `x86_64-apple-darwin` installed. Ensure Cargo and rustc
come from that toolchain rather than a Homebrew installation. CI configures this
through the Rust toolchain action. Native build jobs explicitly select Xcode
16.2 for Swift 6 rather than relying on the runner default.

## Pull requests and repository settings

The `checks.yml` workflow builds native resources and the frontend, tests the
core and desktop, validates synchronized versions and Changesets, and writes an
advisory release plan to the Actions summary. Fork PRs run with read-only tokens
and without release signing credentials. Missing changesets are advisory because
documentation and test-only PRs do not always need a release.

The official [Changesets GitHub App](https://github.com/apps/changeset-bot) is
separate from CI and the release-PR workflow. Install it for this repository to
get PR comments and links for adding changesets. It needs read access to code and
metadata and write access to issues/PRs; select only this repository.

Repository settings use squash-only merges and delete merged branches. Once the
repository is public (or has a plan supporting private branch protection), run:

```sh
bash scripts/protect-main.sh
```

This requires PRs, up-to-date `macOS checks`, resolved conversations, and linear
history; blocks force pushes/deletion; applies to administrators. No second
reviewer is mandatory for a solo-maintainer repository. Add reviewer requirements
when collaborators join. Run the check workflow successfully before protection.
GitHub currently rejects branch protection for this private repository's plan;
the script preserves visibility and never upgrades the account.

For the planned pre-launch history reset, temporarily disable protection if it
has already been enabled, then restore it afterward. Committing/pushing the
workflow files is required before any CI or release-PR automation becomes live.

## Normal release

```sh
npm run changeset
```

Choose the desktop package, bump size, and describe the user-visible change.
Changesets produces a release PR containing the version and changelog. Its custom
version script synchronizes `package.json`, both Rust app manifests, Tauri config,
and lockfile metadata. Website and standalone account-server versions remain separate.

Review the generated release PR and approve its Actions run if GitHub marks it
as requiring approval. Bot-created PR checks use this normal GitHub approval
step; the version workflow never approves or merges its own PR. Wait for
`macOS checks` to pass, then squash-merge the release PR. The macOS workflow:

1. Checks version consistency and runs core tests/frontend build.
2. Builds a universal app, including native permission, cursor, and Wi-Fi helpers.
3. Signs all nested executable images and bundles with Developer ID and hardened
   runtime; verifies the expected Apple team.
4. Notarizes and staples the app, then creates, signs, notarizes, and staples a DMG.
5. Signs the DMG and channel appcast with Sparkle's separate update key.
6. Publishes GitHub release assets, then commits signed feeds to `updates`.

The GitHub release and Sparkle dialog derive notes from the same Changesets
changelog bundled into the app's offline “What's new” view. Changelog rendering
does not execute raw HTML or fetch remote images.

The landing page's macOS download uses GitHub's permanent
`releases/latest/download/extend.computer-macos-universal.dmg` redirect. Publishing
adds this fixed-name copy of the versioned installer; stable releases become
latest, and alpha/beta/canary are prereleases that never replace latest stable.
No website redeploy or GitHub API request is needed for each release. The alias
has identical bytes/signatures/provenance digest to the versioned DMG. Before
the first public stable release, the link may return 404 as expected.

Feeds retain older entries for OS compatibility. Signed feed validation never
expires into an unsigned fallback. Sparkle verifies archives before extraction.
Framework downloads are pinned to a release and SHA-256 digest.

Release jobs serialize across all channels. Re-running after a feed-publication
failure reuses already published binaries and their original build number.
Published version tags identify immutable artifacts: fix a bad release by making
a new version, not replacing its download. `CFBundleVersion` is the monotonically
increasing release workflow run number; do not reset that counter without migrating
installed clients. The human-readable Changesets version is separate.

Before the first public release, test tags, GitHub releases, feed history, and
source commits may be discarded or squashed. Preserve the Apple and Sparkle
signing credentials. After users install a public build, preserve its feed URL,
update key, and increasing build numbers even if source history is rewritten.

## Build provenance

All workflow actions are pinned to exact commit hashes. Dependency versions
are locked, Sparkle's archive has a pinned SHA-256, and CI uses exact Rust/Node
versions. The hosted macOS image, Xcode/SDK, Swift tools, and signing timestamps
can still change; these builds are not claimed to be byte-for-byte reproducible.

For a public source repository, the release workflow creates GitHub/Sigstore
build provenance for the final signed, notarized, stapled DMG before publishing.
The attestation binds its digest to the source commit and builder workflow.
GitHub uses short-lived Actions identity; no extra persistent signing key is
needed. A downloadable `provenance.sigstore.json` accompanies the installer.

On the current plan, private test releases explicitly skip attestations.
[GitHub requires Enterprise Cloud for private attestations](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations).
Making the source repository public automatically enables mandatory provenance;
this configuration never changes repository visibility. The first public CI
release must validate the live OIDC/attestation flow.

Public publishing fails if attestation creation or verification fails. Repair
runs download and verify the original bundle using the original source commit
in release metadata; they do not create new build claims for downloaded binaries.
A formerly private release without provenance cannot be repaired into an
attested public release: build a new version in public CI instead.

Users can verify a downloaded installer with the GitHub CLI:

```sh
gh attestation verify ./extend.computer-VERSION-universal.dmg \
  --repo DaniGuardiola/extend.computer \
  --signer-workflow DaniGuardiola/extend.computer/.github/workflows/release-macos.yml \
  --source-ref refs/heads/main \
  --deny-self-hosted-runners
```

Add `--source-digest EXPECTED_COMMIT_SHA` to require a particular reviewed source
commit. The published bundle can also be passed with
`--bundle ./provenance.sigstore.json`. Apple and Sparkle signatures continue to
protect installation/update authenticity. Sparkle does not automatically verify
GitHub provenance; this is additional verification for users and release CI.
Provenance does not establish that the source/dependencies are harmless or that
a compromised authorized workflow is trustworthy. Protect main and review
workflow changes when publishing. Independent reproducibility checks can be
added later without replacing this distribution or provenance setup.

## Preview channels


Stable is the normal release channel. Beta, alpha, and canary use separate feeds,
and their GitHub releases are marked as prereleases. A preview build initially
uses its own channel; users can explicitly choose another in Settings → Updates.

For beta or alpha, enter Changesets prerelease mode before preparing the release:

```sh
npm run changeset -- pre enter beta
# Commit this state; add normal changesets and merge the generated release PR.
# To graduate:
npm run changeset -- pre exit
```

Canary uses the same explicit prerelease mechanism with `canary`, giving unique
versions such as `0.3.0-canary.0`. Canary is not published on every commit by
default. A nightly snapshot workflow can be added separately.

The release workflow infers the channel from the prerelease suffix. A manual run
may choose a channel explicitly; mismatched suffixes fail before signing. Stable
feeds cannot contain prerelease versions. Switching channels never automatically
downgrades a build: returning from a preview waits for a later stable build.

## Application lifecycle

Sparkle supplies native update dialogs and the app menu's “Check for Updates…”.
Settings exposes automatic checks, automatic downloads, and channel choice.
Development bundles and unconfigured local bundles do not access release feeds.

Checks are deferred during sharing. If a session starts after downloading an
update, installation waits for the session to finish. Installation reserves the
idle desktop under the same lock used to start sessions, then shuts down listeners
and input resources before Sparkle replaces/relaunches the app. Ordinary quit also
runs shutdown. Native helper approval remains managed by macOS.

## Device compatibility

Connection protocol versions are independent from app SemVer. The `EXTEND05`
bootstrap is followed by an encrypted, bounded compatibility envelope containing
supported protocol range, app version, and feature capabilities. Both endpoints
select their highest common protocol and intersect capabilities. App versions
need not match. Input, cursor, and persistent control requests require negotiated
capabilities on both sender and receiver.

Unsupported ranges fail before consent or input. Paired-device presence displays
“Update required” after a pinned, authenticated compatibility failure. Errors tell users to update;
pairings are preserved. This bootstrap intentionally breaks older development
builds: rebuild both devices before testing this change. Keep the compatibility
envelope additive and the bootstrap stable in future releases. Additive feature
changes should use capabilities; incompatible semantics need a protocol version
bump and, where possible, an adapter for the preceding protocol.

The Wi-Fi helper already advertises its own API (`protocol=2`) and requires a
matching signed broker. Helper compatibility and Apple service approval are
separate from device protocol negotiation.

## First production verification

On a configured signing Mac, the isolated native integration test exercises the
shipping Sparkle bridge without opening the desktop app or its device database:

```sh
python3 scripts/test-updater-macos.py
python3 scripts/test-updater-macos.py --tamper
```

It signs two disposable Cocoa host bundles, serves a signed loopback appcast,
clicks Sparkle's standard dialog, verifies installation waits for the sharing
callback to become idle, and checks the updated host relaunches. The second run
modifies the signed feed and verifies a native error/abort without installation.
Artifacts and the dialog capture remain in the printed temporary directory.
This verifies the bridge lifecycle, not the desktop app's permission or account
state migration. Requires Apple signing credentials and exported Sparkle keys
in the local signing directory (or `--credential-directory`).

Local development verification cannot prove Developer ID/notarization or a real
Sparkle replacement. Before public rollout, test two genuinely signed/notarized
builds through a staging feed and then the production feed:

- Install from a downloaded DMG into `/Applications` on Apple Silicon and Intel.
- Perform manual and automatic updates; verify restart and invalid-signature rejection.
- Update while sharing; verify no active session is interrupted and new sharing
  cannot start once installation is committed.
- Verify paired identities, saved credentials, Accessibility/Input Monitoring,
  and Wi-Fi service approvals survive the update.
- Try mixed app/protocol versions and unsupported capabilities; verify clear
  update-required errors without input injection or lost pairings.
- Test channel switching, unavailable feeds, canceled downloads, and retry after
  publishing binaries but before publishing the feed.

---

[← CLI operation](cli.md) · [Contents](../README.md) · [Documentation →](../README.md)
