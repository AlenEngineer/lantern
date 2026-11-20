#!/usr/bin/env bash
# Tests scripts/release-check.sh and scripts/check-changelog.sh on small
# made-up changelogs and changes. CI runs it.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
fails=0

expect() { # NAME WANT GOT
    if [[ $2 == "$3" ]]; then echo "ok   $1"; else echo "FAIL $1: want '$2', got '$3'"; fails=1; fi
}

# release-check.sh reads tags, so it runs in a repo of its own.
rel=$dir/rel
mkdir -p "$rel/scripts"
cp "$repo/scripts/release-check.sh" "$rel/scripts/"
git -C "$rel" init -q
git -C "$rel" -c user.name=t -c user.email=t@t commit -q --allow-empty -m base
git -C "$rel" tag v0.3.0
release() { # VERSION CHANGELOG_BODY
    printf '[workspace]\n\n[workspace.package]\nversion = "%s"\n' "$1" >"$rel/Cargo.toml"
    printf '# Changelog\n\n## [Unreleased]\n\n%s\n## [0.3.0] - 2026-11-13\n\n### Added\n\n- Old.\n' "$2" >"$rel/CHANGELOG.md"
    (cd "$rel" && scripts/release-check.sh 2>/dev/null) | sed -n 's/^release=//p' || true
    (cd "$rel" && scripts/release-check.sh >/dev/null 2>&1) || echo fail
}

expect "a released version is no release" "false" "$(release 0.3.0 '')"
expect "a new version with its section releases" "true" "$(release 0.4.0 $'## [0.4.0] - 2026-11-20\n\n### Added\n\n- New.\n')"
expect "a new version without its section fails" "fail" "$(release 0.4.0 '')"
expect "an undated section isn't enough" "fail" "$(release 0.4.0 $'## [0.4.0]\n')"
expect "a version below the last tag fails" "fail" "$(release 0.2.9 $'## [0.2.9] - 2026-11-20\n')"

changelog() { # BODY: the Unreleased section's contents
    printf '# Changelog\n\n## [Unreleased]\n\n%s\n\n## [0.3.0] - 2026-11-13\n\n### Added\n\n- Old.\n' "$1" >"$dir/CHANGELOG.md"
}

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
