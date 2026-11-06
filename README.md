# chorquest

A native, offline-first presentation engine for live events: worship services,
conferences, classes, anything with a projector and someone running it.

> **Status:** early. This repository currently contains the **engine** only (a
> set of Rust libraries). There is no graphical interface yet.

## Why

Whoever runs the screen at a live event works in two modes, often in the same hour:

- **Planned.** "First the welcome video, then hymn 12, then the announcements."
- **Live.** "The pastor just asked for Psalm 23, find it *now*."

The usual tools each cover one part. One app presents the hymnal, another plays
YouTube, a file manager finds the video, a PDF viewer shows the slides. chorquest
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
sources/
  local/         local folders (music, videos, images, presentations, documents)
  link/          pasted links: YouTube, direct files, web pages
  bible/         offline translations: verse text search + reference parsing
```

The full design is in [ARCHITECTURE.md](ARCHITECTURE.md).

## Try it

Requires Rust 1.99 or newer.

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
See the `chorquest-source-bible` crate docs. `sources/bible/fixtures/sample.tsv`
has a handful of verses to play with.

## Using the engine

```rust
use std::sync::Arc;
use chorquest_engine::{Engine, Query, SourceId};
use chorquest_source_local::{Folder, LocalSource};

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
- [ ] Persisted configuration (folders, sources, stage settings)
- [ ] Sources: YouTube search, web image search, online Bible API, hymnal
- [ ] Multiple outputs (stage + stage-display/confidence monitor + stream)
- [ ] Desktop UI (dashboard + stage window)

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution you
intentionally submit for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
