# Releasing

Releases use local `cargo-release` and `git-cliff`, matching the other CLI
projects. Publishing is a separate maintainer action; normal checks and CI do
not publish to crates.io.

## Prepare

Install `cargo-release` and `git-cliff`, authenticate using `cargo login`, and
start from a clean, current `main` branch with green CI.

Use `just changelog-preview` to review commits since the last release, then
curate the user-facing notes under `## Unreleased` in `CHANGELOG.md`. Commit
those notes before starting the release.

```sh
just check-all
just package
just release-dry-run release
```

Use `release` for the first release at the existing `0.1.0` version. Later,
choose `patch`, `minor`, or an explicit version. Review the archive contents,
generated changelog, version, commit, tag, and publication plan.

## Publish

```sh
just release release
```

This verifies the package, generates the changelog, creates the release commit,
publishes to crates.io, tags the version, and pushes to `origin`. A pushed `v*`
tag triggers the binary-build workflow. It builds Linux x86-64, macOS Apple
Silicon, and Windows x86-64 archives and attaches them to a GitHub release.

Verify the crate installation and the release's archives/checksums after the
workflow completes. Versions published to crates.io cannot be reused. If
publication succeeds but a later step fails, inspect what completed and resume
the missing steps without republishing that version.

The `_changelog-release` recipe prints a preview during dry runs and writes
`CHANGELOG.md` only during an executed release.

The hook only promotes the curated Unreleased notes to the version and UTC
date, leaving a fresh Unreleased heading above them. Previous release entries
are preserved. git-cliff provides the commit preview; release notes remain
manually curated.

For a changelog correction after publication, make a normal follow-up commit.
The corrected file will be included in the next crate version; the existing
release tag and published crate are kept intact.
