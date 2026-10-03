# Release notes

From the repository root, run `npm run changeset` after a user-visible desktop
change. Choose `extend-computer-desktop`, a bump size, and describe the change.
Commit the generated Markdown file with the pull request.

Documentation and test-only changes can omit a changeset. The Changesets bot
comments are advisory; the release workflow prepares version/changelog PRs.

For preview releases, run `npm run changeset -- pre enter alpha` or
`npm run changeset -- pre enter beta`. To graduate, use
`npm run changeset -- pre exit`. The app's Updates settings select the feed.

Website and account-server packages are not in this release workspace and keep
their own versions. Do not edit version numbers manually; the release version
script synchronizes desktop, Rust, Tauri, and lockfile versions together.
