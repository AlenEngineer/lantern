#!/usr/bin/env bash
# Prints the version the CHANGELOG's Unreleased section calls for, or
# nothing when it's empty. Added, Changed, Removed or Deprecated bump the
# minor version; Fixed or Security alone, the patch. (A 1.0 or a major
# bump is a decision, so it's made by hand with scripts/release.sh.)
#
#   scripts/next-version.sh [CHANGELOG [CURRENT_VERSION]]
set -euo pipefail
cd "$(dirname "$0")/.."

changelog=${1:-CHANGELOG.md}
current=${2:-$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' Cargo.toml)}
[[ $current =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || { echo "not a version: $current" >&2; exit 2; }
major=${BASH_REMATCH[1]} minor=${BASH_REMATCH[2]} patch=${BASH_REMATCH[3]}

# The Unreleased section's headings that have at least one entry.
kinds=$(awk '
    /^## \[Unreleased\]/ { on = 1; next }
    /^## \[/ { on = 0 }
    on && /^### / { kind = $2; next }
    on && /^- / && kind != "" { print kind }
' "$changelog" | sort -u)

if grep -qxE 'Added|Changed|Removed|Deprecated' <<<"$kinds"; then
    echo "$major.$((minor + 1)).0"
elif grep -qxE 'Fixed|Security' <<<"$kinds"; then
    echo "$major.$minor.$((patch + 1))"
fi
