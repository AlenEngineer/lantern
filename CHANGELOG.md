# Changelog

All notable changes to this project are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Templates: ☆ makes a saved plan a template; clicking it in the list
  starts a new plan with its order of service, dated today.
- Ctrl+Z takes back the last edit of the plan: a removed, added, moved or
  renamed step, a changed length.

### Fixed

- A long plan name is cut at its edge instead of drawn over the step count.
- Enter on a search that found nothing no longer shows what was looked at
  before, or jumps the plan to the selected step.
- Typing after showing a search result starts a new search instead of
  adding to the old one.
- A step's name or length field kept only the first letter typed after a
  save or open; the rest went to the search box.
- A length or time field takes only digits and a colon; letters stayed in
  it unread.
- Text over the stage's image was hard to read: the image is now dimmed
  under text (**Dim image** in Settings, `dim` in `[stage]`, 60% unless
  set).
- A video, picture or PDF page that doesn't fill the screen has plain bars
  in the background colour beside it, not strips of the stage's image.
- With nothing on stage, On stage shows the stage's image as the projector
  does, instead of "Nothing on the projector"; and blackout is black then
  too, where it showed the image.
- On stage, Up next and Preview show a video's still instead of a black
  box that says "video".
- Passages in the results all had the same unreadable thumbnail; each now
  has a book and says how it starts.
- "Next step · PgDn" was cut off at the bottom of a 720-line window with a
  video on stage: the monitors now shrink to leave the buttons room.
- A title too long for its place ends in "…" instead of being chopped
  mid-letter or running off the pane.
- In a window narrower than about 1140 points the plan and stage panes
  narrow, down to 330 and 280, so the search and results keep some room;
  the bang rows no longer draw over themselves.
- A step's name and a folder's or Bible's path have a line to themselves:
  they showed about twenty characters.
- The plan's rows no longer move when "Saved…" or the readiness chip comes
  and goes (they're under the list now), or when a step goes on stage.
- The circle in the plan's header says "Look"; tooltips are solid, and
  the star's no longer covers the Edit button.
- The plans list counted section headings as steps, so it said "6 steps"
  for a plan whose header says "4 steps".
- PgDn lets go of the selected step, so while the plan runs only the step
  on stage is lit; the pink line then shows where Ctrl+A adds (after the
  current step). Looking at a step of the plan, the button says "Add again".
- Live slot, Black and Note rows have a picture like the others (an icon,
  or black), no longer repeat their name underneath ("Note / note"), and
  their empty length box says "min".
- The plan pane says "Saved “Domingo”" instead of the file's name, and the
  plans list no longer shows its folder's full path.

## [0.7.0] - 2026-11-13

### Added

- A slide strip under On stage: a chip per verse (by its number), stanza,
  page or slide (`17a`, `17b` when one took two); click one to show it.

## [0.6.0] - 2026-11-13

### Added

- The media folders are watched: files added, removed or renamed show up in
  search a second later, even in a search already on screen.

## [0.5.0] - 2026-11-13

### Added

- An output picker: Settings → Output lists the screens and puts the stage
  on the one you pick, now and at every start or plug-in (Hyprland).

## [0.4.0] - 2026-11-13

### Added

- A plan can have its own stage look, saved with it: the swatch in the
  plan's header opens Settings on **This plan**.
- **Go on when done**, per step: a video moves the plan when it ends, an
  image, PDF, passage or black screen after its length (shared among its
  slides), with a countdown on its row. Off by default.
- Plan steps can be dragged to move them: the list opens a gap where they'll
  land and a card follows the pointer. A section brings its steps and lands
  only between sections. Esc lets go without moving anything.
- A folder can have its own bang (`!h` for the hymns), set in Settings or
  with `bang` in `config.toml`; `!f` still searches every folder.
- A double click on a result or plan step shows it at once; a single click
  still only lines it up.
- Downloaded YouTube videos unused for 30 days are deleted at startup.
- Thumbnails in the results and the plan: a picture's own image, a PDF's
  first page, a video's still, a YouTube video's still, and a passage or
  song's first slide in the stage look.
- Text too long for one slide at the smallest size is split into more
  slides (at lines, then sentences, then words, about even in length), and
  stepping goes through them.
- A stage look (Settings → Stage, or `[stage]` in the config): background
  colour and image, text colour, font (with installed fonts suggested), and
  text set as large as fits between a smallest and largest size. The
  dashboard's monitors and a sample in the settings show the same look.
- A ready check: Prepare sums up by the plan's name whether everything can
  be shown, and each step that can't says why (missing file, a kind the
  stage can't show, a YouTube download going or failed, an image link not
  downloaded); Run shows it under the plan's name.
- The plan step on stage shows how far in it is ("slide 2 of 5", "page 1
  of 3", "1:22 / 4:26") with a thin bar under its row.
- Path fields in the settings suggest folders (and Bible or colours files)
  as you type, ignoring case and accents; a click or Tab takes one and goes
  inside it.

### Changed

- A video starts playing as it goes on stage, instead of waiting paused.
- One screen instead of Prepare/Run and the live/plan modes. The plan is
  edited in place (click a step for its strip; Ctrl+A adds after it), the
  lower stage box shows what you're looking at or else the next step, and
  **Plans** lists your saved plans. Ctrl+R and Ctrl+P are gone.
- Section headers say their time in words over a bar ("12 min of 15 min
  planned", "6:40 of 15 min · on track"), and a section without one offers
  **Set a time**. New sections start without a time.

### Removed

- YouTube streaming: a video not downloaded yet waits for its download,
  since a stream started frozen or silent.

### Fixed

- A crash when a slide's text was laid out in a box with no height.
- YouTube and image search thumbnails, which never showed.

## [0.3.0] - 2026-11-13

### Added

- Live slots carry a search (`!yt`, `!b juan 3`): reaching one opens the
  search with it, and its row counts what was shown in it.
- Once something's on stage, ← → step its slides even with text left in the
  search box, until you type again.
- docs/ROADMAP.md: what's planned next, core and app.
- The Prepare view (Ctrl+R): the plan as a document with every name, length
  and budget a field, and the launcher beside it adding results after the
  selected step. "Add here" adds sections, live slots, black screens and
  notes; Alt+↑/↓ moves steps. Sections show whether their steps fit their
  budget while you plan.
- Settings in the app (Ctrl+,): media folders, Bibles, the Brave key and the
  colours file. Save writes `config.toml` and applies it without a restart.
- Plan sections with time budgets. Each section's header shows its time
  against its budget and whether it's on track, tight or over; the launcher
  shows the running section's time left and when the service should end.
  Step lengths come from the operator, the source, the loaded video, or a
  guess.
- YouTube results carry their length (`Resource::DURATION`).
- `colors` in the config names a Matugen-style JSON file of colour roles;
  the dashboard follows it and recolours when it changes.

### Changed

- In Run, clicking a plan step lines it up in the preview instead of putting
  it on stage; Enter shows it and moves the plan there. Editing the plan
  moves to Prepare.
- The dashboard has three panes: the plan, a launcher (search and results),
  and the stage with the preview or the plan's next step. A "live" or "plan"
  chip says which mode you're in; Ctrl+P switches.
- Before the service starts, Ctrl+A adds to the end of the plan instead of
  the start, so a plan built in order keeps it.
- A typed bang turns into a chip beside the search box instead of staying
  in it next to its chip; Backspace in the empty box takes the chip away.
- Typing always goes to the search box. The shortcuts that were letters move
  to Ctrl: Ctrl+B blackout, Ctrl+A add to plan, Ctrl+Space play/pause. ← →
  step only while the search box is empty, and Esc clears it.
- The look follows Material colour roles, with Inter type and Material
  Design Icons. See docs/DESIGN.md.

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
