# Architecture

This document explains how lantern is put together and why. Read it before
making structural changes. Update it when you make one.

## Goals and non-goals

**Goals**

- **One place for planned and live presenting.** Steps prepared ahead of
  time and on-the-fly searches are first-class and mix freely.
- **The core manages media, not media types.** Indexing, searching,
  ordering and staging don't care whether something is a hymn, a YouTube
  video or a PDF. Knowledge of specific sources lives in extensions.
- **Fast and offline-first.** Local content is indexed in memory. The
  network is optional, and a slow source never delays a fast one.
- **The audience only sees what the operator chose.** The stage is a
  separate output with its own state.
- **Native.** An in-process library, no server.

**Non-goals (for now)**

- Rendering. The engine decides *what* is on stage. Drawing it (decoding
  video, rasterizing PDFs, embedding YouTube) is the UI's job.
- Networked or multi-operator control.
- Editing media (slide authoring, video trimming).

## The big picture

```
                     ┌───────────────────────── Engine ─────────────────────────┐
                     │                                                          │
  Source (local) ──┐ │  ┌──────────── Searcher ────────────┐                    │
  Source (hymnal) ─┼─┼─▶│ scan ─▶ Index (in memory)        │──▶ Hits            │
  Source (link) ───┤ │  │ search ─▶ live sources (threads) │──▶ SearchStream    │
  Source (youtube)─┘ │  └──────────────────────────────────┘                    │
                     │                                                          │
                     │  ┌──── Session (orchestrator) ────┐   ┌─── Showman ────┐ │
     Dashboard  ◀───▶│  │ Plan: [Show][Live][Blank]...   │──▶│ preview        │ │
     (operator)      │  │ cursor, log                    │   │ program, media │─┼──▶ Stage window
                     │  └────────────────────────────────┘   │ blackout       │◀┼─── (audience)
                     │                                       └────────────────┘ │  report(id, ..)
                     └──────────────────────────────────────────────────────────┘
```

Dependency graph (arrows point to dependencies):

```
engine ──▶ searcher ──────▶ resource ◀── sources/local
   │                          ▲   ▲ ▲
   ├──────▶ orchestrator ─────┘   │ └─── sources/link
   └──────▶ showman ──────────┘   └───── sources/bible
```

`resource` is the only crate everybody depends on, and it has no internal
dependencies. Sources depend only on `resource`. The engine never depends on a
concrete source. The application decides which sources to register.

## Core concepts

### Resource (`crates/resource`)

A `Resource` is anything that can go on stage. It is a **self-contained
snapshot**: id, title, subtitle, `Kind`, `Location`, tags and free-form
`meta`. Plans store resources by value, so a saved plan keeps working even if
the source that produced it is gone or has been re-indexed.

- **`Kind`** says *how to present* it: `Image`, `Video`, `Audio`, `Document`,
  `Presentation`, `Web`, `Text`, or `Other(String)` for extensions. A YouTube
  video and an `.mp4` are both `Video`.
- **`Location`** says *where the content is*: `File(path)`, `Url(string)`, or
  `Inline(text)` for lyrics, verses and announcements. In inline text, blank
  lines separate slides.
- **`ResourceId`** is `"<source-id>:<key>"`. The prefix tells the engine
  which source owns the resource. The key must be stable across scans.

### Source, the extension point

```rust
pub trait Source: Send + Sync {
    fn id(&self) -> &SourceId;
    fn name(&self) -> &str;
    fn capabilities(&self) -> Capabilities;           // { scan, search }
    fn scan(&self) -> Result<Vec<Resource>, SourceError>;
    fn search(&self, query: &Query) -> Result<Vec<Resource>, SourceError>;
}
```

A source declares one or both capabilities:

| Capability | Called when | Results go to | Good for |
|---|---|---|---|
| `scan` (indexed) | on reindex | the in-memory index | local folders, downloaded hymnal, offline Bible |
| `search` (live) | on every federated search | straight to the caller | YouTube, web search, Bible APIs, pasted links |

The Bible source (`sources/bible`) uses both. One source serves one translation:

- **Scan** emits one resource per verse (`bible-rv1909:John.3.16`, titled
  `Juan 3:16`, with the text inline), so the index finds verses by their
  wording. Verse text is indexed at body weight, so a file whose *title*
  matches still ranks above a verse that only mentions the word.
- **Search** parses references and returns the whole passage as one
  resource with one slide per verse. Book names work in Spanish and English,
  with or without accents, abbreviated (`jn`, `Hch`, `1 Co`), with ordinals
  (`primera de Juan`, `II Reyes`) or `San`. Unambiguous prefixes work too.
  Several references can be joined with `;`. A trailing `:` or `-` while the
  operator is still typing doesn't break parsing.
- Book data and reference parsing (`Book`, `Reference`) are public. A future
  online-API source can reuse them and only fetch the text.

### Searcher (`crates/searcher`)

**Index.** An inverted index in a `BTreeMap<term, postings>`.

- Text is *folded* (Unicode NFD, combining marks removed, lowercased), so
  `cancion` matches *Canción*, and then split into words.
- Each posting stores the weight of the field where the term was found:
  title 3, subtitle/tags 2, inline body 1.
- Every word of the query must match a term. An exact match scores ×2. A
  prefix match (a range scan over the sorted map) scores ×1. Only if
  neither finds anything, a typo match (one edit, including swapped
  letters) scores ×0.5.
- A query that starts the title, or appears in it, gets a bonus.
- An empty query returns everything that passes the filters, so the
  dashboard can browse e.g. "all videos".

At church-library scale (tens of thousands of items) this stays well under
a millisecond per keystroke and needs no external search engine.

**Federated search.** `Searcher::search(query)` returns a `SearchStream`:

1. The index answers synchronously and is always the first event.
2. Each live source runs on its own thread, and its results arrive as soon as
   it finishes. A failure is reported as an event without affecting the others.
3. Every search has a monotonically increasing *ticket*. The UI drops streams
   that are no longer the latest, so late results from a slow source can't
   overwrite newer ones.

Scores are only comparable within one batch. The UI shows batches as
groups ("Local", "YouTube"...), not one merged list.

**Reindexing** scans all indexed sources in parallel *without* holding the
index lock, then swaps each source's entries in. If a scan fails, the
source's previous entries are kept, so unplugging a drive doesn't empty the
library mid-service.

### Orchestrator (`crates/orchestrator`)

A **`Plan`** is an ordered list of **`Step`**s with stable `StepId`s. Each
step has a label, private notes, and an **`Action`**:

- `Show { resource }`: put this on stage.
- `Live { hint, shown }`: a slot for searching on the fly. `hint` is an
  optional preset query (e.g. "videos only"). `shown` collects what was
  presented during the slot.
- `Blank`: clear the stage.
- `Note`: a reminder for the operator. The stage is left alone.

Plans serialize to JSON.

A **`Session`** is a plan being run. Its cursor is a `StepId`, not an index,
so the plan can be edited mid-event (add a song, reorder) without losing your
place. Every resource that goes on stage is appended to a **log**, marked
planned or live. The log can later produce song-usage reports.

The two working modes meet here:

- **Planned → live.** Reach a `Live` step and search. Whatever you show is
  recorded in the slot.
- **Live → planned.** "Add after current" turns something you just found
  into a step.
- **Detours.** Show anything at any time with `show_now`. The cursor
  doesn't move, and `resume` puts the current step back on stage.

### Showman (`crates/showman`)

Showman is the model of the audience window. It doesn't draw anything.

- **Program**: what the audience sees.
- **Preview**: what the operator has lined up. `take()` moves it to
  program. This is the safety net for live search.
- **Blackout**: hides the stage without forgetting the program.
- **Scenes**: a `Scene` is a resource on stage, with a `SceneId` and a
  `Media`. An id is never reused, and `take()` gives the scene a fresh id
  as it moves from preview to program.
- **Media**: one of three shapes. `Frames` has a current frame and a count
  (stanzas, pages, slides). `Timed` has a playback state, a position and a
  duration. `Failed` means the stage can't present the scene. The Showman
  guesses the shape from the resource, and the stage window corrects it.
- **Playback**: timed media starts *paused*, because pressing play should be
  deliberate. Only the stage can report `Ended`. From there, play replays
  from the start and a seek returns to paused.
- **Reports**: a renderer sends a `MediaReport` about one scene, by id: a
  page count, a timeline, a position, the end, or a failure. A report for a
  scene that has left the stage is ignored. Inline text ignores `Frames` and
  `Timeline` reports, because the Showman splits it into frames itself.

Windows don't poll. `subscribe()` returns a `StageFeed`: a snapshot and a
receiver made under one lock, so no event is lost or counted twice. A window
applies each `StageEvent` to its copy with `StageState::apply`. That is the
same function the Showman uses to change its own state, so the copy always
equals the original. A report that changes a scene arrives as a `Media`
event that carries the result. Events are serde-serializable, so they can
cross into a webview or another process unchanged.

### Engine (`crates/engine`)

The engine is what a UI embeds. It owns the `Searcher`, the `Session` and
the `Showman`, and keeps them consistent. It doesn't re-export `Showman`, so
the engine is the only way to change its stage.

- `next()` / `prev()` behave like a presentation clicker: next frame if
  there is one, otherwise next step. Reaching a step *performs* it
  (`Show` → stage, `Blank` → clear, `Live`/`Note` → nothing).
- `preview` / `take` / `show_now` handle off-plan content.
- `clear`, `clear_preview`, `set_blackout`, `toggle_blackout`,
  `goto_frame`, `play`, `pause`, `toggle_playback` and `seek` are the other
  operator controls.
- `report(id, report)` is how a stage window tells the engine what it
  learned about a scene.
- `stage()` returns a snapshot, and `subscribe()` returns a `StageFeed`.

Every operator method runs through one private function, `operate`. It
takes the locks in order and compares the program's `SceneId` before and
after the action. If a new scene is on program, `operate` logs it, as
planned when it came from the plan and as live otherwise.

## Threading model

- `Engine` is `Send + Sync`. Share one `Arc<Engine>` across the UI.
- State lives behind `Mutex`/`RwLock`. Critical sections are short and never
  do I/O. The lock order is `session` before `stage`, and `Engine::operate`
  is the only place that takes both.
- `Engine::report` takes only the stage lock, so a stage window never waits
  behind `save_plan`.
- Stage events go out on std `mpsc` channels. A UI thread that can't block
  moves the feed's receiver to a small thread of its own.
- Blocking work happens only in `Source::scan` (during reindex, on scoped
  threads) and `Source::search` (one thread per live source per search).
- Sources are **synchronous** on purpose (see decisions below). A source that
  needs HTTP can use a blocking client. It already runs on its own thread.

## Design decisions

These are short ADRs. Add new ones at the end.

1. **Libraries, not services.** The original sketch had `orchestrator`,
   `searcher` and `showman` as separate binaries. They are now in-process
   crates: a desktop app gains nothing from IPC between its own parts, and
   loses latency and simplicity. The names stayed.
2. **Synchronous sources, threads for concurrency.** Async would force a
   runtime choice on every extension and on the UI. Searches fan out to at
   most a handful of sources, so a thread each is cheap and keeps the trait
   object-safe and simple. This can be revisited if a source needs streaming
   results.
3. **Own index instead of a search engine.** Tantivy and similar crates are
   built for millions of documents on disk. Our corpus is small and lives in
   memory. A ~300-line index that folds Spanish accents and handles
   as-you-type prefixes is faster to start, easier to tune and has no
   on-disk state to corrupt.
4. **Resources are snapshots.** Plans embed full resources instead of ids,
   so a plan opened on another machine, or after a reindex, still has
   everything it needs.
5. **Kind is about presentation, not origin.** This keeps the stage
   independent of sources. A new source never requires stage changes unless
   it introduces a genuinely new way of presenting something.
6. **The stage is a state machine with events.** The engine never knows
   about windows. Any number of outputs can subscribe, which leaves room for
   multiple outputs later.
7. **Accent folding maps `ñ` to `n`.** Search is forgiving by design. A rare
   false positive (`año`/`ano`) is better than missing *Canción* when
   someone types `cancion`.
8. **Renderers report facts about one scene, by id.** The stage window
   knows what the engine can't: a PDF's page count, a video's length, that
   it reached the end, or that it can't play a link. It says so with a
   `MediaReport` tagged with the scene's `SceneId`, and it is the authority
   on how the scene is presented. A late report finds its scene or nothing.
   The engine never encodes what a renderer can play, which keeps ADR 6.
9. **Stage state changes only by applying events.** `StageState::apply` is
   the only code that writes the Showman's state, and every window keeps its
   copy with the same function. `subscribe()` hands out the snapshot and the
   receiver together. So a copy can't drift from the original.
10. **The engine is the only door to its stage, and logging is derived.**
    `with_stage` used to hand out the whole `Showman`, which skipped the log.
    Now each control is an engine method, and `operate` logs whatever new
    scene ends up on program. A new method can't forget to log.

## Writing a new source

1. Create `sources/<name>/` with a crate named `lantern-source-<name>`
   that depends only on `lantern-resource` (plus whatever it needs).
2. Implement `Source`. Pick `Capabilities::INDEXED`, `LIVE`, or both.
3. Build ids with `ResourceId::new(self.id(), key)` and a stable key.
4. Choose the `Kind` by *how it should be presented*. Put source-specific
   data in `meta` and document the keys in the crate docs.
5. For live sources, return `Ok(vec![])` quickly for queries that aren't
   meant for you. Return `SourceError::Unavailable` when offline or not
   configured.
6. Add unit tests that need no network. Put network tests behind
   `#[ignore]`.

## Future work

- **Folder watching**: incremental index updates with `notify`, instead of
  full rescans.
- **Configuration**: a persisted config listing folders and enabled sources.
- **Cancellation**: pass a cancellation token to `Source::search` so
  superseded searches stop early instead of being ignored.
- **Multiple outputs**: named outputs (main stage, stage display for
  musicians showing the next stanza, livestream overlay) with their own
  program and blackout.
- **Thumbnails and durations**: an optional enrichment step after scanning.
