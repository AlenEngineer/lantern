#!/usr/bin/env bash
# Fails a PR that changes the code (anything under crates/ or sources/)
# without a new entry under the CHANGELOG's ## [Unreleased]. A change
# nobody would notice, a refactor or tests, carries the no-changelog
# label instead: CI then sets NO_CHANGELOG=1 and this passes.
#
#   scripts/check-changelog.sh BASE_REF
set -euo pipefail
cd "$(dirname "$0")/.."

base=${1:?usage: $0 BASE_REF}
paths=(crates sources)

if [[ ${NO_CHANGELOG:-} == 1 ]]; then
    echo "Labelled no-changelog: no entry needed."
    exit 0
fi

# A file moved in or out counts on both sides.
changed=$(git diff --no-renames --name-only "$base" -- "${paths[@]}")
if [[ -z $changed ]]; then
    echo "Nothing under ${paths[*]} changed: no changelog entry needed."
    exit 0
fi

unreleased() {
    awk '/^## \[Unreleased\]/ { on = 1; next } /^## \[/ { on = 0 } on && /^- /' "$@"
}
before=$(git show "$base:CHANGELOG.md" | unreleased | sort)
after=$(unreleased CHANGELOG.md | sort)
added=$(comm -13 <(echo "$before") <(echo "$after"))

if [[ -z $added ]]; then
    shown=$(head -n 10 <<<"$changed")
    more=$(($(wc -l <<<"$changed") - 10))
    {
        echo "These files changed, but CHANGELOG.md has no new entry:"
        echo
        echo "  ${shown//$'\n'/$'\n'  }"
        if ((more > 0)); then echo "  ... and $more more"; fi
        echo
        echo "Add one line under ## [Unreleased], under ### Added, Changed, Removed"
        echo "or Fixed, saying what changed for the operator. If nobody would"
        echo "notice (a refactor, tests), add the no-changelog label to the PR"
        echo "instead: the check runs again when it's added."
    } >&2
    if [[ -n ${GITHUB_ACTIONS:-} ]]; then
        echo "::error file=CHANGELOG.md::Code under ${paths[*]} changed but there is no new line under ## [Unreleased]. Add one, or the no-changelog label if nobody would notice."
    fi
    exit 1
fi
echo "New changelog entries:"
echo "$added"
