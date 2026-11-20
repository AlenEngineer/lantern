#!/usr/bin/env bash
# Whether this commit is a release: the version in Cargo.toml's
# [workspace.package] has no vX.Y.Z tag yet. Then it must be higher than
# the latest tag and CHANGELOG.md must have its "## [X.Y.Z] - date"
# section, or this fails. CI runs it on every PR; release.yml runs it on
# main and publishes when it says release=true.
#
#   scripts/release-check.sh [CHANGELOG [CARGO_TOML]]
#
# Prints release=true|false and version=X.Y.Z, also to $GITHUB_OUTPUT
# when that is set.
set -euo pipefail
cd "$(dirname "$0")/.."

changelog=${1:-CHANGELOG.md}
cargo=${2:-Cargo.toml}
version=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' "$cargo")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "::error file=$cargo::not a version: '$version'" >&2; exit 1; }
tag=v$version

out() {
    echo "release=$1"
    echo "version=$version"
    if [[ -n ${GITHUB_OUTPUT:-} ]]; then
        printf 'release=%s\nversion=%s\n' "$1" "$version" >>"$GITHUB_OUTPUT"
    fi
}

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "$tag is already released."
    out false
    exit 0
fi

latest=$(git tag -l 'v[0-9]*.[0-9]*.[0-9]*' | sort -V | tail -n 1)
if [[ -n $latest ]] && [[ $(printf '%s\n%s\n' "$latest" "$tag" | sort -V | tail -n 1) != "$tag" ]]; then
    echo "::error file=$cargo::Version $version is lower than the latest release, $latest." >&2
    exit 1
fi
if ! grep -q "^## \[$version\] - [0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}" "$changelog"; then
    echo "::error file=$changelog::Version $version is new, but $changelog has no '## [$version] - YYYY-MM-DD' section for it." >&2
    exit 1
fi
echo "$tag is new: merging this releases it."
out true
