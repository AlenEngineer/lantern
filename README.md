# lantern

A native, offline-first presentation engine for live events: worship services,
conferences, classes, anything with a projector and someone running it.

> **Status:** early. The engine and a first desktop app (operator dashboard and
> stage window) work. Expect rough edges.

## Why

Whoever runs the screen at a live event works in two modes, often in the same hour:

- **Planned.** "First the welcome video, then hymn 12, then the announcements."
- **Live.** "The pastor just asked for Psalm 23, find it *now*."

The usual tools each cover one part. One app presents the hymnal, another plays
YouTube, a file manager finds the video, a PDF viewer shows the slides. lantern
puts all of that in one place:

- **One search box for everything.** Local folders are indexed in memory, so
  search is instant and works offline. It ignores accents and case, matches
  prefixes and forgives typos. Pasted links (YouTube, images, web pages) work
  right away. Online sources (YouTube search, the web, Bible APIs) plug in as
  extensions.
- **Plans with live slots.** Prepare an ordered list of steps ahead of time,
  and leave *live slots* for the parts you can't plan. Showing something
  off-plan never loses your place.
- **Two windows.** The operator's dashboard and the audience's stage are
  separate. The stage shows only what you *take* to it. A preview lets you
  check a search result before anyone sees it.
- **Native and fast.** No server, no browser required, no account.

## Repository layout

```
crates/
  resource/      shared vocabulary: Resource, Kind, Location, Query, the Source trait
  searcher/      in-memory index + federated search across sources
  orchestrator/  plans (steps, live slots) and the running session
  showman/       stage state: program/preview, frames, playback, blackout
  engine/        facade that wires everything together; what a UI embeds
  app/           the desktop app: operator dashboard + stage window (binary `lantern`)
sources/
  local/         local folders (music, videos, images, presentations, documents)
  link/          pasted links: YouTube, direct files, web pages
  bible/         offline translations: verse text search + reference parsing
```

The full design is in [ARCHITECTURE.md](ARCHITECTURE.md).

## Run the app

Linux builds are attached to each
[release](https://github.com/AlenEngineer/lantern/releases). Building from
source needs Rust 1.99 or newer. Either way, the app needs GStreamer with
its base and good plugins (and `gst-libav` for H.264), `pdftoppm` (poppler)
for PDFs, and `yt-dlp` for YouTube links.

```sh
cargo run --release -p lantern-app
```

Two windows open: the **dashboard** for the operator and the fullscreen
**stage** for the audience. Type in the search box to find files and Bible
verses, type a reference like `jn 3 16-17`, or paste a YouTube link. The
dashboard's bottom line lists the keys. The e2e scripts drive them through
the same key table the keyboard uses, but they haven't been tried on a
physical keyboard yet.

| Key | Does |
| --- | --- |
| Enter | preview the top result, press again to take it to the stage |
| ↑ ↓ | pick a result (previews it) |
| A | add the preview to the plan |
| → / PgDn, ← / PgUp | next / back (stanza, page, or plan step) |
| Space | play / pause |
| B | blackout |
| Esc, / | leave / return to the search box |

YouTube videos download ahead of time (720p H.264) into
`~/.cache/lantern/youtube` as soon as they enter the plan or the preview,
so playback never depends on the network. The dashboard shows each video's
download state.

### Plans

The PLAN panel holds the rundown you build with A. Click its name to rename
it. **Save** (Ctrl+S) writes it to
`~/.local/share/lantern/plans/<name>.json` (or under `$XDG_DATA_HOME`); a
plan that already has a file saves over it, and a new plan never replaces
another one with the same name (`sunday-service-2.json`). A dot after the
name means unsaved changes. **Open** (Ctrl+O) lists the saved plans, newest
first. **New** (Ctrl+N) starts an empty one. Leaving a plan saves it first if
it has a file, and otherwise asks before discarding it. The app reopens the
last plan you opened or saved, and says why if it can't. The Ctrl keys work
while typing in the search box.

### Configuration

The app reads `~/.config/lantern/config.toml`, or the file named by
`LANTERN_CONFIG`. Relative paths are relative to the config file, and `~`
works. Unknown keys are errors, so typos show up on the dashboard.

```toml
[[folders]]
path = "~/Videos"
label = "Videos"          # optional

[[folders]]
path = "/media/church/Music"

[[bibles]]
lang = "es"               # "es" or "en": which reference names to accept
path = "~/bibles/rv1909.json"
name = "RV1909"           # optional
```

Folders and Bibles are indexed in the background at startup. The dashboard
shows the item count, or the error for a source that failed to load.

### Putting the stage on the projector

The stage window has the app id `lantern-stage`, and the dashboard
`lantern-dashboard`. The app can't pick a monitor itself, so ask the
compositor. On Hyprland (0.57, Lua config), move the stage to the
projector's workspace once it is open:

```sh
hyprctl dispatch 'hl.dsp.window.move({ workspace = N, window = "class:lantern-stage" })'
```

The e2e harness uses this command. It has not been tried with a real
projector yet, and a permanent window rule for your compositor is the
better fix.

### End-to-end screenshots

`scripts/e2e.sh <script> <outdir>` runs the real app on two headless
Hyprland outputs, follows a script from `scripts/e2e/`, and saves a
screenshot of both windows at every `shot`. `basic`, `media`, `library`,
`dashboard`, `plans` and `youtube` (needs the network) cover hymns, images,
video, PDFs, Bible passages, the dashboard, plan files and YouTube.

```sh
scripts/e2e.sh scripts/e2e/dashboard.script ~/.cache/lantern/e2e/dashboard
```

Each run saves plans in a fresh `<outdir>/data`, never in your own. `reopen`
checks that the next start reopens the plan `plans` saved, so it shares its
data folder:

```sh
scripts/e2e.sh scripts/e2e/plans.script ~/.cache/lantern/e2e/plans
E2E_DATA=~/.cache/lantern/e2e/plans/data \
    scripts/e2e.sh scripts/e2e/reopen.script ~/.cache/lantern/e2e/reopen
```

## Try the engine alone

```sh
cargo test --workspace
cargo run --example demo -- ~/Music ~/Videos ~/Pictures
cargo run --example demo -- --bible es:sources/bible/fixtures/sample.tsv ~/Music
```

The demo is a small terminal "dashboard". A background thread acts as the
stage window and prints what the audience would see. Type a word to search,
paste a YouTube link, or type a reference like `jn 3 16-17`. Then try
`:pre 0`, `:take`, `:add 0`, `:n`, `:plan`, or `:help` for everything else.

### Bible translations

No translation text is bundled, because most modern translations are
copyrighted. Use a file you're allowed to use: Reina-Valera 1909 and the King
James Version are public domain. The source reads a JSON layout (an array of
66 books, each with `chapters`) and a simple TSV (`book⇥chapter⇥verse⇥text`).
See the `lantern-source-bible` crate docs. `sources/bible/fixtures/sample.tsv`
has a handful of verses to play with.

## Using the engine

```rust
use std::sync::Arc;
use lantern_engine::{Engine, Query, SourceId};
use lantern_source_local::{Folder, LocalSource};

let engine = Arc::new(Engine::new());
engine.add_source(
    LocalSource::new(SourceId::new("local")?, "Media")
        .with_folder(Folder::new("/media/church/Videos"))
        .with_folder(Folder::new("/media/church/Music").labeled("Música")),
)?;
engine.reindex();

let feed = engine.subscribe(); // the stage window starts from feed.state

let hits = engine.search_index(&Query::new("cancion"));
engine.preview(hits[0].resource.clone()); // operator checks it
engine.take();                            // audience sees it
engine.next();                            // next stanza or next step
```

## Roadmap

- [x] Core model, in-memory index, federated search
- [x] Plans with live slots, session log
- [x] Stage model (program/preview, frames, playback, blackout)
- [x] Sources: local folders, pasted links, Bible (offline, Spanish/English references)
- [ ] Watch local folders for changes (no manual reindex)
- [x] Configuration file (folders, Bibles)
- [x] Desktop UI (dashboard + stage window): images, video, PDFs, Bible, YouTube
- [ ] Hymnal source (hymns are a built-in sample today)
- [ ] Web pages on the stage
- [ ] Choose the stage monitor from the app
- [ ] Sources: YouTube search, web image search, online Bible API
- [ ] Multiple outputs (stage + stage-display/confidence monitor + stream)

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution you
intentionally submit for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
