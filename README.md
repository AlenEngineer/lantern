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
- **Plans with live slots.** Build an ordered list of steps ahead of time,
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
  youtube/       YouTube search through yt-dlp (!yt, on request)
  brave/         image search through the Brave Search API (!gi, on request)
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
**stage** for the audience. The dashboard is three cards on one
ground. Across the top, a **strip**: the plan's name and its buttons, the
running section's time and how the service stands against its schedule,
the time of day, then Search, what isn't ready, the stage's screen, Blackout
and Settings. Each says how something stands and opens what changes it. On
the left, the **plan**, a line a step. In the middle, the **stage**: what
the audience sees, large, and beside it what comes next, or what you're
looking at. Ctrl+K drops the **search** from the strip over that side
column, with the result you're looking at under the list; Esc sends it
back. Drag the gap between the plan and the stage, or between the live
picture and the column beside it, to make them narrower or wider; a double
click lets them size themselves again. The widths are kept.

There is one screen, the same before and during the service. The plan is
edited where it is: the selected step's row carries small buttons to rename
it, move it and remove it (and to go on by itself), F2 turns its name into
a field, and a click on its length, or a section's time, turns that into
one; Enter or Esc is done. Ctrl+Enter adds the result you're looking at right
after it, where the pink line shows while you search.
**Add step** at the end of the plan adds a **Section**, **Live slot**,
**Black** or **Note**. The **‹** beside the plan's name (Ctrl+O) goes to
your plans: the one that's open, your templates, **New plan** and the
saved ones. A new plan starts on its name, and with no plan to reopen
Lantern starts on that list. **Save** shows while the plan has unsaved
changes.

Results show a thumbnail: a picture, a PDF's first page or a video's still
(YouTube too). A passage has a book instead, and its result says how it
starts. A plan step is one line: its mark (● on stage, ○ next, else its
number), its name, and at its end the one thing worth saying of it: what's
wrong with it, how far the stage is into it, what kind it is, or how long
it takes (↦ after it means it goes on by itself). Selecting it opens a
second line with what can be done to it.

Ctrl+K opens the search, and while it's open typing goes to its box, so
every shortcut is a key that doesn't type. Esc empties the box, and closes
the search once it's empty. Find files and Bible verses, type a reference like
`jn 3 16-17`, or paste a YouTube link. The column beside the live picture shows
what you're looking at (a result, or a step you clicked), and otherwise the
plan's next step: Enter shows the first, PgDn the second. A single click
on a result or step only lines it up; a double click shows it at once.

| Key | Does |
| --- | --- |
| Enter | look at the top result, press again to show it; with the box empty, show the selected step |
| Tab | complete a half-typed bang |
| ↑ ↓ | pick a result (previews it) |
| ↑ ↓ with no search | move through the plan's steps, or your plans in the list; Enter shows the step or opens the plan |
| PgDn, PgUp | next / back: slide, page, or plan step (what clickers send) |
| click a chip under On stage | jump to that verse, stanza, page or slide (`16`, `17a`) |
| → ← | the same, while the search box is empty, or once something's on stage until you type again |
| Ctrl+Enter | add the result you're looking at after the selected step |
| Alt+Enter | keep it on standby instead: ready under the plan, not in its order |
| Ctrl+Space, play key | play / pause |
| Ctrl+B | blackout |
| Ctrl+K | open the search |
| Esc | clear the search box, keeping its bang chip for the next search; again closes the search |
| Del | with no search, remove the selected step |
| F2 | rename the selected step, in its row |
| Backspace | in an empty box, take the bang chip away |
| Alt+↑ ↓ | move the selected step (or drag it: the list opens a gap where it'll land; a section brings its steps; Esc lets go) |
| Ctrl+Z | take back the last edit of the plan (a removed step comes back) |
| Ctrl+S, Ctrl+O | save the plan, list your plans (Esc goes back) |
| Ctrl+N | in the list of plans, a new plan; in a plan, **Add step**: ↑ ↓ choose the kind, Enter adds it |
| Ctrl+, | settings: folders, Bibles, image search key, colours |

### Changing keys

Every key in the table can be another: under **Keys** in Settings, click a
key and press the new one (Esc gives up, ↺ puts the usual key back). A
letter needs Ctrl or Alt, or it would type. In the config a key is written
like `ctrl+t`, `alt+up`, `f9` or `pagedown`. Buttons and hints show
the keys you chose. In `config.toml` it's the `[keys]` table, holding only
what differs from the usual:

```toml
[keys]
new = "ctrl+t"
blackout = "f9"
```

### Running a plan

The plan moves when you move it: PgDn (or a clicker) goes to the next
slide, then the next step. Sections are headings; their times only report.
A video starts playing as it goes on stage, and Ctrl+Space pauses it.

A step can go on by itself: select it and turn on **Go on when done**. A
video then moves the plan when it ends; an image, PDF, passage or black
screen after its length (shared among its pages or slides), and its row
counts down. Sections, live slots and notes always wait for you.

### Bangs

Start the search with a bang to ask only one kind of source, like Brave's
search bangs. An empty box lists them; Tab or Enter completes one. Once
typed with a space after it, the bang turns into a chip beside the box,
which keeps only what you're looking for; Backspace in the empty box takes
the chip away again.

| Bang | Searches |
| --- | --- |
| `!b` | the Bibles: verse text, or a reference like `!b jn 3 16` |
| `!f` | the media folders |
| `!h`, … | one folder, by the bang you gave it in Settings |
| `!yt` | YouTube, through `yt-dlp` |
| `!gi` | images on the web, through the [Brave Search API](https://brave.com/search/api/) (needs a key) |

`!yt` and `!gi` go over the network and take a few seconds, so they search
when you press Enter instead of as you type, and a plain search never asks
them. A YouTube result previews and downloads like a pasted link. An image
is downloaded into `~/.cache/lantern/images`, so pasted image links work
too. Image search uses SafeSearch strict.

Give a folder its own bang in Settings (the field after its path) or with
`bang = "h"` in `config.toml`, and `!h himno` searches only that folder;
folders may share a bang. `!f` still searches every folder. A bang Lantern
already uses (`b`, `f`, `gi`, `t`, `x`, `yt`) can't be taken.

Every source declares its own bangs (`Source::bangs`), so a new source
brings its own.

The strip has a button for each bang, so nobody has to remember them: a
book for the Bible, a play mark for YouTube, a folder, a picture, a clock,
a pencil for your own text. A folder's own bang shows as it's typed
(`!h`). A button only opens the search on its bang, exactly as typing it
would, and its tooltip says which bang that is. They show in a window
1100 wide or more.

The plan's YouTube videos (its steps and its standby shelf) download ahead
of time (720p H.264) into `~/.cache/lantern/youtube` as soon as they enter
the plan, and the dashboard shows each one's download state. They usually
finish long before they're needed, so the service doesn't depend on the
network.

A video shown outside the plan, searched and thrown on screen, plays
straight from YouTube and leaves nothing on disk. It starts a few seconds
after you look at it, can be paused, scrubbed and replayed like any other,
and needs the network for as long as it plays. If it can't be streamed it's
downloaded instead, and adding it to the plan downloads it. Lantern plays
one fixed video stream and one audio stream side by side; the adaptive
stream it tried before changed quality as it went, which froze the picture
and dropped the sound. Settings → Library → **YouTube** switches this off
(`always_download = true`), and then every video is downloaded first.

The open plan's pictures and PDF pages are loaded when it opens and kept
while it's open, the standby shelf first, so any of them shows at once in
any order; an image at a link is downloaded then too. They're let go when
they leave the plan or another plan opens. Decoded pictures are kept up to
512 MB (about sixty at full HD); past that, the rest load when they're
next, as videos always do: a video's player is only made for the one
lined up and the next step's.

The cache is plain `.mp4` files, safe to delete. A video unused for 30 days
(not played, previewed or in the open plan) is deleted when Lantern starts.

### Plans

The plan pane holds the rundown you build with Ctrl+Enter. Click its name to
rename it. **Save** (Ctrl+S) writes it to
`~/.local/share/lantern/plans/<name>.json` (or under `$XDG_DATA_HOME`); a
plan that already has a file saves over it, and a new plan never replaces
another one with the same name (`sunday-service-2.json`). "unsaved" next
to the name means it has changes since. **Open** (Ctrl+O) lists the saved plans, newest
first. **New** (Ctrl+N) starts an empty one. Leaving a plan saves it first if
it has a file, and otherwise asks before discarding it. The app reopens the
last plan you opened or saved, and says why if it can't.

**Templates.** The ☆ beside a saved plan makes it a template, such as
"Domingo" with its sections, times, live slots and notes. Templates sit at
the top of the list; clicking one starts a new, unsaved plan from it, titled
with today's date ("Domingo 2026-11-15"), with everything but what its live
slots showed last time. **Edit** opens the template itself.

Before the service starts, Ctrl+Enter adds to the end of the plan; once it has
started, right after the current step.

**Standby.** Some things you want ready without knowing when, or whether,
they'll be needed: a spare song, a notice to move a car. Alt+Enter puts
the result you're looking at (or, with no search, what's lined up, such as
the selected step's) on the plan's standby shelf, under its steps. A click
there lines it up and a second click or Enter shows it; ↓ past the last
step reaches the shelf too. Showing from standby doesn't move the plan:
PgDn goes on from where it was. ✕, or Del while it's lined up, takes it
off. The shelf is saved with the plan and comes along with a template.

**Your own text.** `!x Bienvenidos` shows what you typed. ` | ` starts a
new line and ` || ` a new slide, since the search box has one line:
`!x Feliz sábado | a todos || Dios les bendiga` is two slides. Like
anything else it can be shown, added to the plan or kept on standby.

**The verse in the corner.** While a Bible passage is on stage, its lower
right corner says where it is and in which Bible ("Juan 3:17 · RVG"),
following the verse as you step through. It sits in the margin the text
leaves free. Settings → Stage → **Verse** hides it (`no_reference = true`
under `[stage]`), for all plans or for one with a look of its own.

**Time on stage.** `!t` finds what shows time: **Clock** (the time of
day), **Timer** (counts up from when it's shown) and countdowns. `!t 7` is a
seven-minute countdown, `!t 2:30` one of two and a half minutes, and also
one to half past two. A plain search for "clock", "timer" or "countdown 7"
finds them too. They're shown, added to the plan or kept on standby like
anything else; a countdown counts from the moment it goes on stage, stops at
0:00, and in a plan has its length, so a step set to go on by itself moves
on when it runs out. Their figures are set several times larger than a
passage's text, in the stage's font and colours.

**Stage look.** Settings → Stage sets what the audience sees: a background
colour and image (covering the screen, also shown when the stage is clear;
blackout stays black), the text colour, the font (type to find an installed
one), and a size range. Text is set as large as fits between the two sizes,
so a short line is big and a long passage shrinks to fill the screen. Text
that doesn't fit even at the smallest size is split into more slides, at
its lines, then sentences, then words, about even in length; PgDn and the
arrows step through them like verses. A
sample shows the look as you type, and the dashboard's monitors are true
miniatures of the stage. In `config.toml` it's the `[stage]` table:

```toml
[stage]
background = "#1b2a3a"
image = "~/Pictures/cross.jpg"
text = "#fff4d6"
font = "DejaVu Serif"
min_size = 40   # points on a 1080-line screen
max_size = 96
dim = 60        # % the image is dimmed under text; 0 leaves it as it is
```

Where text is set over the image, the image is dimmed toward the background
colour so the words read; an empty stage shows it as it is.

A plan can have a look of its own, for a Santa Cena or a youth night: the
**⋯** in the plan's header opens Settings on **This plan**, where
**Give it its own look** starts from the look for all plans. The same place
has the plan's name, **Make it a template**, **Duplicate** (saves the plan
and opens a copy) and **Delete** (press twice; Ctrl+Z right after brings the plan back).

**Its folders**, in the same place, lists folders of media only that plan
needs, such as a campaign's. They're searched like your media folders
while the plan is open, and no longer once another plan is. They're saved
in the plan, written as you typed them (`~/Church/Campaña`). The look is
saved in the plan too, so opening the plan anywhere brings it, and **Use
the look for all plans** removes it. Opening another plan while something
is on stage leaves that as it looks: the new plan's look starts with the
next thing shown.

**Starts at** is when the service begins, on a 24-hour clock (`10:30`).
With it, each section still to come says when it's due ("10:45 · 12 of 15
min planned"), counting each section before it for its time, or without
one for its steps. The header counts down to the start ("starts 10:30 · in
12 min") and, once the service is under way, says how far it runs from that
schedule ("4 min behind", "2 min ahead", "on time") in place of "ends in".
A section counts for its whole time even when its steps are shorter, so a
late start reads as behind until time is made up.

**Export**, beside them, puts the plan and everything it shows into one
file, `Sábado.lantern` in your Documents, to take or send to another
computer: the files its steps and standby shelf show, its own folders
whole, its look's image, and its YouTube videos as downloaded. What can't
be found is left out and named. Exporting again replaces the file. It's a
zip with nothing compressed, so any archive tool opens it. On the other
computer, **Import a plan** at the end of the list of plans shows the
exported plans in Documents, Downloads, the desktop and on anything plugged
in, with a field for one somewhere else. Importing copies the media into
Lantern's own folder (`bundles/` beside `plans/`), so the USB stick can
come out, and opens the plan; if a plan of that name is already saved, the
new one is named apart ("Sábado (imported)"). Deleting an imported plan
removes its media the next time Lantern starts, once no copy of it uses
them.

**Its Bibles** picks which of your Bibles the plan searches: all of them
until you pick one. A plan that names a Bible this computer doesn't have
searches them all.

**Ready check.** While something in the plan isn't ready, a line under its
name says so ("2 not ready yet", "1 problem"), and each step that isn't says
why: a file that's missing, something the stage can't show, a YouTube
download still going (amber) or failed (red), an image link not downloaded
yet.

**Live slots** are the parts of a service you'll fill on the spot ("the
director's song choice"). Give one a search in its strip (`!yt`, `!b`, or a
whole query like `!b juan 3:16-18`); when the service reaches it, the
search box opens with it, ready for Enter. Its row says how much was shown
in it.

Clicking a step selects it and lines it up to look at; Enter (or **Show
now**, or a double click) puts it on stage and moves the plan there, so
nothing reaches the projector from a stray single click.

**Sections and time.** Split the plan into parts with a time budget, such as
"Alabanza, 15 min" and "Palabra, 30 min": a new section offers **Set a
time**, or click it and type its time (`15` is fifteen minutes, `4:30` four
and a half). Each section header says in words how it's doing, "12 min of
15 min planned" over a bar while you plan, "6:40 of 15 min · on track"
while it runs. Once a section starts, its header shows the time
against its budget, and the strip shows how much is left, or how
far over it is, plus when the service should end. It warns, it never stops
you: "on track", "tight" when it should end in the last minute or tenth of
its budget, "over" past it. Each step's length is the one you typed, else
the one its source gives (a YouTube video's), else its real length once
loaded, else a guess shown as `~4 min`. A playing video counts what's left
of it exactly.

### Configuration

**Settings** (Ctrl+, or the gear in the strip) edits the config from the
app: media folders, Bibles, the Brave key and the colours file. **Save**
(Ctrl+S) writes the file and applies it at once: the folders and Bibles are
reindexed and the colours reload, no restart. It rewrites the file, so
comments in it are not kept. Path fields suggest what's on disk as you
type, like a shell: an empty field lists your home folder, `musica` finds
*Música*, and clicking a suggestion or Tab takes it and goes inside.

The media folders are watched: a file added, removed or renamed shows up in
search (and in a search already on screen) about a second later, without
a reindex. A folder that isn't there when Lantern starts, such as an
unplugged drive, isn't watched until the next save or start.

The app reads `~/.config/lantern/config.toml`, or the file named by
`LANTERN_CONFIG`. Relative paths are relative to the config file, and `~`
works. Unknown keys are errors, so typos show up on the dashboard.

```toml
# Settings go before the first [[folders]] or [[bibles]], or TOML reads
# them as part of that table.
brave_api_key = "…"       # optional, for !gi image search
colors = "~/.config/amane/colors.json"  # optional, see below
stage_output = "HDMI-A-1"  # optional: the stage's screen, see below

[[folders]]
path = "~/Videos"
label = "Videos"          # optional
bang = "v"                # optional: !v searches only this folder

[[folders]]
path = "/media/church/Music"

[[bibles]]
lang = "es"               # "es" or "en": which reference names to accept
path = "~/bibles/rv1909.json"
name = "RV1909"           # optional
```

`!gi` needs a Brave Search API key: sign up at
[brave.com/search/api](https://brave.com/search/api/), which includes free
monthly credit, and paste the key into the config. Keep the config file
private, since the key is billed to you.

Folders and Bibles are indexed in the background at startup. The dashboard
says so while it indexes, and names any source that failed to load.

The dashboard's colours are Material colour roles (`primary`, `surface`,
`on_surface_variant`…). `colors` names a JSON file of them in the format
[Matugen](https://github.com/InioX/matugen) templates write, so a desktop
themed from its wallpaper can theme Lantern too; the file is read again
whenever it changes. Without it, Lantern uses Material's baseline dark
palette. [docs/DESIGN.md](docs/DESIGN.md) says which role goes where.

Settings → Appearance → **Glass** lets the desktop show through the
dashboard: **See-through** goes from solid to 80%, and follows the slider
as it moves. **Blur** has the desktop blur what shows through, which is
what keeps text readable over a busy wallpaper. How strong the blur is,
and whether there is any, is the desktop's to say: Lantern can only draw
its own window. On Hyprland the switch works at once; on other desktops
that blur behind windows it takes effect when Lantern next starts. The
stage window is always solid. In the config they're `see_through = 40`
and `no_blur = true`.

### Putting the stage on the projector

Settings → **Output** lists the screens (`HDMI-A-1 · Epson · 1920×1080`).
Picking one moves the stage there at once; Save keeps it as
`stage_output` in the config. Lantern then puts the stage there when it
starts, and when that screen is plugged in later. It doesn't fight you
otherwise: a stage moved by hand stays where it was put.

This needs Hyprland for now: iced can't list monitors or fullscreen on a
chosen one, and on Wayland only the compositor places windows, so Lantern
asks Hyprland (`hyprctl`). Elsewhere, move the window yourself: the stage's
app id is `lantern-stage`, the dashboard's `lantern-dashboard`.

### End-to-end screenshots

`scripts/e2e.sh <script> <outdir>` runs the real app on two headless
Hyprland outputs, follows a script from `scripts/e2e/`, and saves a
screenshot of both windows at every `shot`. `basic`, `media`, `library`,
`dashboard`, `launcher`, `plans`, `timer`, `settings` (run it with
`LANTERN_CONFIG` pointing at a copy: it saves), `youtube` and `online`
(both need the network) cover hymns, images, video, PDFs, Bible passages, the dashboard,
bangs, plan files, YouTube downloads and search, and image links.

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

## Releases

Every feat or fix PR adds a line under `## [Unreleased]` in
`CHANGELOG.md`; CI fails it otherwise (`scripts/check-changelog.sh`). On
`main`, a **release PR** (`chore(release): vX.Y.Z`) keeps itself up to
date with the next version and its notes: anything added, changed or
removed bumps the minor version, fixes alone the patch
(`scripts/next-version.sh`). Merging it tags the version and publishes a
Linux build on the releases page. A 1.0 or other major version is made by
hand with `scripts/release.sh X.Y.Z`.

## Roadmap

- [x] Core model, in-memory index, federated search
- [x] Plans with live slots, session log
- [x] Stage model (program/preview, frames, playback, blackout)
- [x] Sources: local folders, pasted links, Bible (offline, Spanish/English references),
  YouTube search, image search
- [x] Watch local folders for changes (no manual reindex)
- [x] Configuration file (folders, Bibles)
- [x] Desktop UI (dashboard + stage window): images, video, PDFs, Bible, YouTube
- [x] Launcher dashboard with search bangs (`!b`, `!f`)
- [x] `!yt` YouTube search, `!gi` image search (Brave), image links on stage
- [x] Per-folder bangs (`!h` for a hymns folder)
- [ ] Hymnal source (hymns are a built-in sample today)
- [ ] Web pages on the stage
- [x] Choose the stage's output from the app (Hyprland)
- [ ] Sources: online Bible API
- [ ] Multiple outputs (stage + stage-display/confidence monitor + stream)

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution you
intentionally submit for inclusion in this work, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
