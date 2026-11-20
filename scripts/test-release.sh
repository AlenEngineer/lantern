#!/usr/bin/env bash
# Tests scripts/next-version.sh and scripts/check-changelog.sh on small
# made-up changelogs and changes. CI runs it.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
fails=0

expect() { # NAME WANT GOT
    if [[ $2 == "$3" ]]; then echo "ok   $1"; else echo "FAIL $1: want '$2', got '$3'"; fails=1; fi
}

changelog() { # BODY: the Unreleased section's contents
    printf '# Changelog\n\n## [Unreleased]\n\n%s\n\n## [0.3.0] - 2026-11-13\n\n### Added\n\n- Old.\n' "$1" >"$dir/CHANGELOG.md"
}

next() { "$repo/scripts/next-version.sh" "$dir/CHANGELOG.md" "$1"; }

changelog ''
expect "nothing unreleased, no version" "" "$(next 0.3.0)"
changelog $'### Fixed\n\n- A crash.'
expect "fixes only bump the patch" "0.3.1" "$(next 0.3.0)"
changelog $'### Added\n\n- A thing.\n\n### Fixed\n\n- A crash.'
expect "an addition bumps the minor" "0.4.0" "$(next 0.3.2)"
changelog $'### Changed\n\n- Videos start on their own.'
expect "a change bumps the minor" "1.3.0" "$(next 1.2.9)"
changelog $'### Added\n\n### Fixed\n\n- A crash.'
expect "an empty heading doesn't count" "0.3.1" "$(next 0.3.0)"

# check-changelog.sh compares against a base commit, so it needs a repo.
git -C "$dir" init -q
git -C "$dir" -c user.name=t -c user.email=t@t commit -q --allow-empty -m base
changelog ''
git -C "$dir" add CHANGELOG.md
git -C "$dir" -c user.name=t -c user.email=t@t commit -q -m changelog
cp "$repo/scripts/check-changelog.sh" "$dir/"
mkdir -p "$dir/scripts" && mv "$dir/check-changelog.sh" "$dir/scripts/"
check() { (cd "$dir" && NO_CHANGELOG=${1:-} scripts/check-changelog.sh HEAD >/dev/null 2>&1) && echo pass || echo fail; }

expect "nothing changed needs no entry" pass "$(check)"
mkdir -p "$dir/docs" && echo x >"$dir/docs/a.md"
git -C "$dir" add docs
expect "docs need no entry" pass "$(check)"
mkdir -p "$dir/crates/app" && echo x >"$dir/crates/app/a.rs"
git -C "$dir" add crates
expect "code without an entry fails" fail "$(check)"
expect "code labelled no-changelog passes" pass "$(check 1)"
git -C "$dir" rm -q --cached -r crates && rm -r "$dir/crates"
mkdir -p "$dir/sources/bible" && echo x >"$dir/sources/bible/a.rs"
git -C "$dir" add sources
expect "a source without an entry fails" fail "$(check)"
git -C "$dir" -c user.name=t -c user.email=t@t commit -q -m code
git -C "$dir" mv sources/bible/a.rs a.rs
expect "a file moved out of the code counts" fail "$(check)"
changelog $'### Added\n\n- Drag to reorder.'
expect "code with an entry passes" pass "$(check)"

# The workflows that run all this, when actionlint is installed (it also
# runs shellcheck on their scripts when that is).
if command -v actionlint >/dev/null; then
    if actionlint "$repo"/.github/workflows/*.yml; then echo "ok   workflows lint clean"; else fails=1; fi
else
    echo "skip workflows lint: actionlint isn't installed"
fi

if command -v shellcheck >/dev/null; then
    if shellcheck "$repo"/scripts/*.sh; then echo "ok   scripts lint clean"; else fails=1; fi
else
    echo "skip scripts lint: shellcheck isn't installed"
fi

exit $fails
