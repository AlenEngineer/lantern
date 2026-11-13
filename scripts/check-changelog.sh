#!/usr/bin/env bash
# Fails a feat or fix PR that doesn't add an entry under the CHANGELOG's
# Unreleased section. Other kinds (docs, ci, chore, refactor, test, style,
# perf) pass. The PR title is the squash commit's subject, so it decides.
#
#   scripts/check-changelog.sh BASE_REF "feat(app): the PR title"
set -euo pipefail
cd "$(dirname "$0")/.."

base=${1:?usage: $0 BASE_REF TITLE}
title=${2:?usage: $0 BASE_REF TITLE}

if ! [[ $title =~ ^(feat|fix)(\(.*\))?!?: ]]; then
    echo "\"$title\" isn't a feat or fix: no changelog entry needed."
    exit 0
fi

unreleased() {
    awk '/^## \[Unreleased\]/ { on = 1; next } /^## \[/ { on = 0 } on && /^- /' "$@"
}
before=$(git show "$base:CHANGELOG.md" | unreleased | sort)
after=$(unreleased CHANGELOG.md | sort)
added=$(comm -13 <(echo "$before") <(echo "$after"))

if [[ -z $added ]]; then
    cat >&2 <<EOF
"$title" is a ${BASH_REMATCH[1]}, but CHANGELOG.md has no new entry under
## [Unreleased]. Add one line saying what changed for the operator, under
### Added, Changed, Removed or Fixed.
EOF
    exit 1
fi
echo "New changelog entries:"
echo "$added"
