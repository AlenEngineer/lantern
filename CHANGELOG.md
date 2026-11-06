# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `chorquest-resource`: `Resource`, `Kind`, `Location`, `Query` and the `Source` extension trait.
- `chorquest-searcher`: in-memory index with accent folding, prefix and typo matching;
  federated search streaming results from live sources.
- `chorquest-orchestrator`: plans with show/live/blank/note steps, JSON persistence,
  sessions with an edit-safe cursor and a log of everything shown.
- `chorquest-showman`: stage model with preview/program, frames, playback and blackout,
  plus serializable events for the stage window.
- `chorquest-engine`: facade tying everything together, plus a terminal demo.
- `chorquest-source-local`: local folders source.
- `chorquest-source-link`: pasted links source (YouTube, direct files, web pages).
- `chorquest-source-bible`: offline Bible source. Loads translations from JSON or TSV,
  indexes every verse, and parses Spanish/English references (`jn 3 16-18`,
  `1 Co 13:4-7`, `primera de Juan 4`) into passages with one slide per verse.

### Changed

- Minimum supported Rust version is now 1.99.
