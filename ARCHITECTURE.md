# Architecture

This document explains how chorquest is put together and why. Read it before
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
     (operator)      │  │ cursor, log                    │   │ program, frame │─┼──▶ Stage window
                     │  └────────────────────────────────┘   │ blackout       │ │    (audience)
                     │                                       └────────────────┘ │
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
- **Frames**: a `Scene` is a resource plus a frame index (stanza, page,
  slide). Inline text knows its frame count. For documents and decks, the
  stage window reports the count once it has opened the file.
- **Playback**: timed media starts *paused* (pressing play should be
  deliberate) and has play/pause/seek.

Windows don't poll. They `subscribe()` and receive `StageEvent`s. A window
that opens late starts from `state()`. Events are serde-serializable, so they
can cross into a webview or another process unchanged.

### Engine (`crates/engine`)

The engine is what a UI embeds. It owns the `Searcher`, the `Session` and
the `Showman`, and keeps them consistent:

- `next()` / `prev()` behave like a presentation clicker: next frame if
  there is one, otherwise next step. Reaching a step *performs* it
  (`Show` → stage, `Blank` → clear, `Live`/`Note` → nothing).
- `preview` / `take` / `show_now` handle off-plan content and log it.
- `with_stage` exposes everything else on the stage (blackout, playback,
  frame counts).

## Threading model

- `Engine` is `Send + Sync`. Share one `Arc<Engine>` across the UI.
- State lives behind `Mutex`/`RwLock`. Critical sections are short and never
  do I/O. The lock order is `session` before `stage`.
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

## Writing a new source

1. Create `sources/<name>/` with a crate named `chorquest-source-<name>`
   that depends only on `chorquest-resource` (plus whatever it needs).
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
