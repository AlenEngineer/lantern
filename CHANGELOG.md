# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] - 2026-11-12

### Added

- Search bangs: `!b juan 3` asks only the Bibles, `!f` only the media
  folders. An empty search box lists them, and Tab completes one. Sources
  declare their bang through `Source::bang`.
- `!yt` searches YouTube through `yt-dlp`. Results preview and download
  like a pasted link.
- `!gi` searches images through the Brave Search API, with SafeSearch
  strict. It needs `brave_api_key` in the config.
- Sources can be on request (`Capabilities::on_request`): only their bang
  asks them, and the dashboard waits for Enter before searching them.
- The stage shows images from links, downloaded into
  `~/.cache/lantern/images`, falling back to a source's thumbnail.
- A YouTube video needed before its download is done streams from YouTube's
  HLS playlist while the download goes on, instead of showing black until
  the file arrives. Streams can't be scrubbed or replayed.

### Changed

- The dashboard is a launcher: one search box, one list (results, bangs, or
  the plan with Ctrl+P), the preview beside it, and what's on stage in a bar
  along the bottom.

## [0.1.0] - 2026-11-06

### Added

- `lantern-app`: the desktop app (binary `lantern`), an iced dashboard window and a
  fullscreen stage window that follows the engine.
- `lantern-app`: the stage shows text, images, local video and PDF pages, reports page
  counts, timelines, positions, the end and failures back to the engine, shows black for
  anything it can't show, and preloads the preview and the next plan step.
- `lantern-app`: YouTube links download ahead of time with yt-dlp into
  `~/.cache/lantern/youtube` and play from there; the dashboard shows the download state.
- `lantern-app`: `~/.config/lantern/config.toml` (or `LANTERN_CONFIG`) lists media
  folders and Bibles, indexed in the background at startup.
- `lantern-app`: the operator dashboard, with search results grouped by source, preview
  and take, the plan with its current step, a transport with a timeline and the ended
  state, blackout, and keyboard shortcuts.
- `lantern-app`: save, open and start plans from the dashboard (Ctrl+S, Ctrl+O, Ctrl+N),
  rename a plan by clicking its name, see unsaved changes, and reopen the last plan at
  startup. Plans live in `~/.local/share/lantern/plans`.
- Releases: `scripts/release.sh X.Y.Z` versions and tags, and a pushed tag publishes a
  GitHub release with a Linux build and its changelog notes.
- `scripts/e2e.sh`: runs the real app on headless Hyprland outputs from a script and
  screenshots both windows. Plans go to a fresh data folder per run, or `E2E_DATA`.
- `lantern-resource`: `Resource`, `Kind`, `Location`, `Query` and the `Source` extension trait.
- `lantern-searcher`: in-memory index with accent folding, prefix and typo matching;
  federated search streaming results from live sources.
- `lantern-orchestrator`: plans with show/live/blank/note steps, JSON persistence,
  sessions with an edit-safe cursor and a log of everything shown.
- `lantern-orchestrator`: `Plan::default()`, an empty "Untitled" plan.
- `lantern-showman`: stage model with preview/program, frames, playback and blackout,
  plus serializable events for the stage window.
- `lantern-showman`: `SceneId` on every scene, and `Media` (`Frames`, `Timed` or `Failed`)
  for how it is presented. Timed scenes can reach `Playback::Ended`.
- `lantern-showman`: `MediaReport`, so a stage window can report a page count, a timeline,
  a position, the end, or a failure for one scene.
- `lantern-showman`: `StageState::apply`, the reducer the Showman and every window share,
  and `StageState::scene` to find a scene by id.
- `lantern-engine`: facade tying everything together, plus a terminal demo.
- `lantern-engine`: `report`, plus `clear`, `clear_preview`, `set_blackout`, `toggle_blackout`,
  `goto_frame`, `play`, `pause`, `toggle_playback` and `seek`.
- `lantern-engine`: the demo has `:seek SECS`, `:frames N` and `:end`.
- `lantern-source-local`: local folders source.
- `lantern-source-link`: pasted links source (YouTube, direct files, web pages).
- `lantern-source-bible`: offline Bible source. Loads translations from JSON or TSV,
  indexes every verse, and parses Spanish/English references (`jn 3 16-18`,
  `1 Co 13:4-7`, `primera de Juan 4`) into passages with one slide per verse.

### Changed

- Minimum supported Rust version is now 1.99.
- CI runs on `main` and pull requests: the whole workspace on Linux with GStreamer, and
  the engine crates on Windows and macOS.
- `subscribe` returns a `StageFeed`, the snapshot and the receiver taken under one lock.
- `Showman::take` returns `bool`.
- Every stage action through the engine is logged when it puts a new scene on program.

### Removed

- `Engine::with_stage`. Use the engine's stage methods.
- The `Showman` re-export from `lantern-engine`.
- `Showman::set_frame_count`. Use `report` with `MediaReport::Frames`.
- The `Scene` fields `frame`, `frame_count` and `playback`, and `Playback::Static`. Use `Scene::media`.
- `Kind::is_timed`. A stage decides what it can play and reports it.
