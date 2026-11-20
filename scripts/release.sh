#!/usr/bin/env bash
# Writes a release PR's one commit: sets the version in Cargo.toml's
# [workspace.package] (every crate inherits it), and moves the CHANGELOG's
# Unreleased entries under "## [X.Y.Z] - today". Run it on a new branch,
# then open the PR. Merging it releases: release.yml sees the new version,
# tags it and publishes the builds (scripts/release-check.sh).
#
#   git switch -c release/v0.8.0 origin/main
#   scripts/release.sh 0.8.0
#
# The version is a choice: before 1.0, a minor version for anything added
# or changed, a patch for fixes alone.
set -euo pipefail

version=${1:?usage: $0 X.Y.Z}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $version" >&2; exit 2; }
cd "$(dirname "$0")/.."

[[ $(git branch --show-current) != main ]] || { echo "make a branch for the release PR first" >&2; exit 1; }
[[ -z $(git status --porcelain --untracked-files=no) ]] || { echo "commit or stash your changes first" >&2; exit 1; }
! git rev-parse -q --verify "refs/tags/v$version" >/dev/null || { echo "v$version already exists" >&2; exit 1; }
grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "CHANGELOG.md has no Unreleased section" >&2; exit 1; }

sed -i "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$version\"/" Cargo.toml
cargo update --workspace --quiet
sed -i "s/^## \[Unreleased\]/## [Unreleased]\n\n## [$version] - $(date +%F)/" CHANGELOG.md

scripts/release-check.sh >/dev/null
git commit --quiet -am "chore(release): v$version"
echo "Committed v$version. Push the branch and open its PR; merging it releases."
