#!/usr/bin/env bash
# Prepares a release: sets the workspace version, turns the CHANGELOG's
# Unreleased section into this version's, commits, and tags. Nothing is
# pushed; the script prints the command that publishes it.
#
#   scripts/release.sh 0.2.0
#
# Usually the release PR does this instead (.github/workflows/release-pr.yml):
# it runs `scripts/release.sh X.Y.Z --prepare` on its own branch, which
# commits without tagging, and merging the PR tags and publishes.
set -euo pipefail

version=${1:?usage: $0 X.Y.Z [--prepare]}
prepare=${2:-}
[[ -z $prepare || $prepare == --prepare ]] || { echo "unknown option: $prepare" >&2; exit 2; }
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $version" >&2; exit 2; }
tag=v$version
cd "$(dirname "$0")/.."

[[ -n $prepare || $(git branch --show-current) == main ]] || { echo "release from main" >&2; exit 1; }
[[ -z $(git status --porcelain --untracked-files=no) ]] || { echo "commit or stash your changes first" >&2; exit 1; }
! git rev-parse -q --verify "refs/tags/$tag" >/dev/null || { echo "$tag already exists" >&2; exit 1; }
grep -q '^## \[Unreleased\]' CHANGELOG.md || { echo "CHANGELOG.md has no Unreleased section" >&2; exit 1; }

# The one version line in [workspace.package]; every crate inherits it.
sed -i "/^\[workspace.package\]/,/^\[/ s/^version = \".*\"/version = \"$version\"/" Cargo.toml
cargo update --workspace --quiet

sed -i "s/^## \[Unreleased\]/## [Unreleased]\n\n## [$version] - $(date +%F)/" CHANGELOG.md

git commit --quiet -am "chore(release): $tag"
[[ -n $prepare ]] && exit 0
git tag -a "$tag" -m "lantern $tag"
echo "Tagged $tag. Publish it with:"
echo "  git push origin main $tag"
