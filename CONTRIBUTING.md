# Contributing

Thanks for helping. This is a small project, so the rules are few but they matter.

## Before you start

- Read [ARCHITECTURE.md](ARCHITECTURE.md). If your change doesn't fit the
  design described there, open an issue first so we can discuss it.
- New *sources* (a Bible API, a hymnal, a stock-photo site...) are the
  easiest and most welcome contributions. See "Writing a new source" in the
  architecture document.

## Workflow

1. Branch from `main`: `feat/<topic>`, `fix/<topic>`, `docs/<topic>`, `chore/<topic>`.
2. Keep each commit focused. Use [Conventional Commits](https://www.conventionalcommits.org/):
   `feat: index local folders`, `fix(searcher): keep entries when a scan fails`.
3. Before pushing, run:

   ```sh
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

   CI runs the same checks.
4. Update `CHANGELOG.md` under *Unreleased* if users or extension authors
   would notice the change.
5. Open a pull request describing *why*, not only what.

## Code guidelines

- No `unsafe` (enforced by `#![forbid(unsafe_code)]` through workspace lints).
- Public items have doc comments (`missing_docs` warns).
- No panics on user input or I/O errors. Return errors, and skip bad files
  instead of failing a whole scan.
- Tests must not need the network. Mark tests that do with `#[ignore]`.
- Add a dependency only when it pays for itself. Add it to
  `[workspace.dependencies]` and refer to it with `.workspace = true`.
- Error messages are lowercase and have no trailing period.
