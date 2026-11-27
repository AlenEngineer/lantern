# Contributing

Thanks for helping. This is a small project, so the rules are few but they matter.

## Before you start

- Read [ARCHITECTURE.md](ARCHITECTURE.md). If your change doesn't fit the
  design described there, open an issue first so we can discuss it.
- New *sources* (a Bible API, a hymnal, a stock-photo site...) are the
  easiest and most welcome contributions. See "Writing a new source" in the
  architecture document.

## How a change gets merged

One request is one PR, squash-merged. `main` takes no direct pushes: a ruleset requires a PR
with the `check` job of `.github/workflows/ci.yml` (it waits for every CI job) and the
`changelog entry` check passing. Agents run the whole sequence below with the `ship` skill.

1. **Order.** Nothing comes first; say in the PR if the work depends on another PR.
   Planned work is in [docs/ROADMAP.md](docs/ROADMAP.md).
2. **Branch** from `main`: `feat/<topic>`, `fix/<topic>`, `docs/<topic>`, `chore/<topic>`.
   Keep each commit focused and use [Conventional Commits](https://www.conventionalcommits.org/):
   `feat: index local folders`, `fix(searcher): keep entries when a scan fails`.
3. **Check locally what you touched**: `cargo fmt --all`, then
   `cargo clippy -p <crate> --all-targets -- -D warnings` and `cargo test -p <crate>` for each
   crate you changed (`--workspace` when you changed something many crates use). A change to
   the app's screens runs `scripts/e2e.sh` on the matching script in `scripts/e2e/`. CI runs
   the full suite: the whole workspace on Linux, the app on Windows, and the engine crates
   (everything but `lantern-app`, which needs GStreamer) on Windows and macOS.
4. **Update `CHANGELOG.md`** under *Unreleased*: CI requires a new line there for any change
   under `crates/` or `sources/`. If users or extension authors wouldn't notice it (a
   refactor, tests), add the `no-changelog` label to the pull request instead; that re-runs
   only the changelog check.
5. **Open the PR** from the template (`.github/pull_request_template.md`), describing *why*,
   not only what. The title is one line a user would understand, because the release notes
   are written from the titles. A user-visible change updates the README in the same PR.
6. **Two reviews by two different agents, neither the PR's author**, posted on the PR:
   behaviour (CI's release build, the `lantern-linux` or `lantern-windows` artifact, against
   main's, trying to break what the PR claims) and design (the `design-reviewer` agent,
   against [ARCHITECTURE.md](ARCHITECTURE.md), and [docs/DESIGN.md](docs/DESIGN.md) for
   the app's look). A must-fix from either blocks the merge. Fixes are new commits on the
   same PR, and both reviews run again. A pushed branch is never rebased or force-pushed;
   bring `main` in with a merge commit. Should-fix items become issues.
7. **Merge** with a squash once CI is green and both reviews have no must-fix. No one else
   has to approve it. The owner can say "hold" on any PR, and then it waits.

## Releasing

Versions follow [Semantic Versioning](https://semver.org/). Every crate
shares the one version in `[workspace.package]`.

`scripts/changelog-draft.sh` lists the PRs merged since the last tag. Before a release,
check that *Unreleased* covers each of them, and give it a short paragraph on what matters,
then Added, Changed, Fixed, with anything breaking in bold.

1. On a new branch from an up-to-date `main`, run
   `scripts/release.sh X.Y.Z`. It sets the version, moves the changelog's
   *Unreleased* entries under `## [X.Y.Z] - date`, and commits.
2. Open a PR from it. CI's *release check* makes sure the version goes up
   and has its changelog section.
3. Merge it. On `main`, `release.yml` sees the new version, runs CI, tags
   `vX.Y.Z` and publishes a GitHub release with a Linux and a Windows
   build and that version's changelog notes.

## Code guidelines

- No `unsafe` (enforced by `#![forbid(unsafe_code)]` through workspace lints).
- Public items have doc comments (`missing_docs` warns).
- No panics on user input or I/O errors. Return errors, and skip bad files
  instead of failing a whole scan.
- Tests must not need the network. Mark tests that do with `#[ignore]`.
- Add a dependency only when it pays for itself. Add it to
  `[workspace.dependencies]` and refer to it with `.workspace = true`.
- Error messages are lowercase and have no trailing period.
