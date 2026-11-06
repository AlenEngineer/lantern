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
- `chorquest-showman`: `SceneId` on every scene, and `Media` (`Frames`, `Timed` or `Failed`)
  for how it is presented. Timed scenes can reach `Playback::Ended`.
- `chorquest-showman`: `MediaReport`, so a stage window can report a page count, a timeline,
  a position, the end, or a failure for one scene.
- `chorquest-showman`: `StageState::apply`, the reducer the Showman and every window share,
  and `StageState::scene` to find a scene by id.
- `chorquest-engine`: facade tying everything together, plus a terminal demo.
- `chorquest-engine`: `report`, plus `clear`, `clear_preview`, `set_blackout`, `toggle_blackout`,
  `goto_frame`, `play`, `pause`, `toggle_playback` and `seek`.
- `chorquest-engine`: the demo has `:seek SECS`, `:frames N` and `:end`.
- `chorquest-source-local`: local folders source.
- `chorquest-source-link`: pasted links source (YouTube, direct files, web pages).
- `chorquest-source-bible`: offline Bible source. Loads translations from JSON or TSV,
  indexes every verse, and parses Spanish/English references (`jn 3 16-18`,
  `1 Co 13:4-7`, `primera de Juan 4`) into passages with one slide per verse.

### Changed

- Minimum supported Rust version is now 1.99.
- `subscribe` returns a `StageFeed`, the snapshot and the receiver taken under one lock.
- `Showman::take` returns `bool`.
- Every stage action through the engine is logged when it puts a new scene on program.

### Removed

- `Engine::with_stage`. Use the engine's stage methods.
- The `Showman` re-export from `chorquest-engine`.
- `Showman::set_frame_count`. Use `report` with `MediaReport::Frames`.
- The `Scene` fields `frame`, `frame_count` and `playback`, and `Playback::Static`. Use `Scene::media`.
- `Kind::is_timed`. A stage decides what it can play and reports it.
