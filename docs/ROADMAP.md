# Roadmap

What's planned next, split by where the work lives: the **core** (the
`lantern-*` crates: resources, plans, the stage model, the engine), which
any front end can use, and the **app** (the iced dashboard and stage
window). Each item says what it needs from each side.

## Stage appearance

Done: one stage look for the whole app (Settings → Stage, `[stage]` in the
config): background colour and image, text colour, font, and text set as
large as fits between a smallest and largest size, measured by the
renderer at the real size. A plan can carry its own look (`Plan::look`). Text that still doesn't fit is split into more
slides. Still to do below.

**Themes with layers.** *(background colour and image done, app-wide)* A stage theme is a stack of layers, drawn bottom to
top: a background (colour, image or looping video), the content, and
optional overlays (a lower third with the reference or song title, a logo).
- Core: a `StageTheme` in the stage model, saved with the plan (a default,
  overridable per step), so a saved plan looks the same anywhere. The stage
  events carry which theme is active.
- App: the stage window renders the layers; the plan gets a theme editor;
  Settings holds the default theme.

**Fonts.** The family and a size range per theme. *(done, app-wide)*
- Core: `font`, `max_size`, `min_size` in the theme.
- App: load the family (system fonts through iced), pick it in the editor.

**Text that fits.** *(done)* The size is an upper limit: the text grows to fill its
box up to `max_size`, and shrinks for long text down to `min_size`. Text
that still doesn't fit at `min_size` is split into more slides, at line or
sentence breaks, and those slides step like verses.
- Core: slides are already frames; splitting needs a measure of the text,
  which only the renderer has. The renderer reports how many slides a
  text needed (like it reports a PDF's page count), and the stage model
  steps through them. Stays testable with a fake measure.
- App: measure with iced's text layout at the stage's real size, binary
  search the size between the limits, then split.

## Stepping through passages

`juan 3 1-16` already gives one slide per verse, stepped with PgDn/PgUp and
←/→. The arrows only step while the search box is empty, though, and after
a search it isn't, so stepping feels impossible.
- App (done with the live slots): once something goes on stage, ← → step
  its slides even with text in the search box, until you type again; the
  search stays, so a slow `!yt` search isn't lost.
- App: the slide strip of what's on stage (verse numbers), click one to
  jump to it. *(done)*

## Plan

- **Live slots** carry a search (`!yt`, `!b`, a folder's bang); reaching one
  opens the search ready, and what was shown is kept in the slot. *(done)*
- **Progress** in each plan row: slide or page of what's on stage. *(done)*
- **Ready check** before the service: downloads done, files present, kinds
  the stage can show, image links downloaded. *(done; reading every PDF
  ahead is still to do)*
- **Templates:** start a plan from a saved order of service. *(done)*

Left over from the Plan mode rebuild, against its mockup
(`.audit/plan-mode-rebuild/mockup-render.png`, from
`.audit/plan-mode-rebuild/mockup.html`), checked with
`scripts/e2e/gaps.script`:

- **A step's time of day** left of its dot, from the plan's start and the
  lengths before it, only when the plan has a start time. *(done)*
- **A text step's picture:** its first slide drawn small; the filmstrip
  draws each slide so. *(done)*
- **The library's soft warning** is the ⚠ on the picture only, its words
  on hover. *(done)*
- The preview's picture fills its frame; the plan's title line is at the
  top of the plan. *(done)*
- Still unlike the mockup: a step's second line says only its kind
  ("text") where the mockup has where it is from and how many slides
  ("himnario · Himno 2 · 4 slides").

## Phone remote

The phone page follows `docs/DESIGN.md` like the dashboard: cards on a
darker ground, states as tints (on stage `tertiary`, next `primary`,
blackout `error`), solid `primary` only for play and the timeline, pills,
Inter, Material Design Icons. It is for Run and Live only.

- **Plan mode:** the phone says the computer is planning, with the plan's
  name and its start time counting down, and offers nothing to press.
- **Strip:** a pill across the top with the mode's chip (Run, Live), the
  plan's name and the volume pill. The section's pace sits under it in its
  own tint ("tight", "over"), as on the computer.
- **Run, stage first:** the stage card (picture, ● live, title, the
  timeline with the play button, or the slides as a row of numbers), what
  is next with ○ and its picture, then only Prev and a wide Next (in the
  next tint) in the thumb's reach.
- **The menu rises from the transport** (swipe up or tap its handle) as a
  panel with the strip's shoulders upside down: Blackout (error tint) and
  Clear at its top, then the plan with the desktop's markers, done rows
  dimmed, sections with their pace. Tap a row to pick it; **Show now**
  takes it.
- **A blackout is lifted in one tap:** while the stage is black, a
  "Blackout on · Lift" pill sits above Next, outside the menu.
- **Volume as a panel** dropped from the strip's volume pill: the output's
  name, a slider with − and + steps of 5%, and Mute. The pill says
  "muted" in the error tint while it is.
- **Live starts from sources, not bangs.** Bangs are for the keyboard;
  the phone opens on a tile for each source (the Bible, each folder under
  its own name, YouTube, Text, Time, Images) under a search of
  everything. A tile sends its bang for you. Each opens a picker made
  for fingers:
  - **Bible:** book (Old and New Testament grids), then chapter, then
    verse, each grid sized from the translation; the steps so far stay
    as pills on top. A passage is two taps, first and last verse, with
    its text shown under the grid before it goes up.
  - **A folder:** every folder added in Settings → Library is a tile,
    under its label. All folders share one plain screen: a search of
    that folder (its bang, sent for you) over its files listed in
    natural order, so `004` comes before `010`. Only the sources Lantern
    ships get pickers of their own; a user's folder never does.
  - **Time:** Clock, Timer, the usual countdowns and the next few
    quarter hours as times to count to: what `!t` makes, as buttons.
  - **Text** a box and Show; **YouTube** and **Images** a search with
    pictures.
  Under the tiles, "Shown this session", each row showing its item again.
- **Live is tap, then Send.** No Next button: a tapped result opens a
  **Send to the stage** bar under it (on-stage tint), and tapping the
  same result again sends it too. Holding it lines it up instead, as the
  dashboard's standby does (Alt+Enter). What is on stage is stepped
  (a passage's verses, a deck's slides) with ˄ ˅ beside the small stage
  card at the top.
- **One result row for every kind, on the phone and the computer:** a
  16:9 picture on the left, its size in the picture's lower right corner,
  its state in the upper left only when it has one, then the title and
  one line of where it's from and what it is, or the one warning that
  matters in its tint (amber shows but badly, red won't show).
  - Video: first frame, length; ⚠ when smaller than the screen.
  - Online (YouTube, links, web pictures): ☁ until downloaded, ↓ and a
    percentage with a bar while downloading, ● live for a stream that
    needs the internet throughout. Lantern can't measure the
    connection, so it says where a thing comes from, not how fast.
  - Picture: itself, its pixels; ⚠ and "smaller than the screen" when it
    is smaller than the stage's output.
  - Deck: first slide, "12 slides", the videos inside. PDF: first page,
    "8 pages".
  - Sound: the source's icon, length. Passage: its first words, "3
    verses", how many slides. Time: the time itself.
  - Missing: ⚠ in red, and where it was.
- **Three layouts for results**, switched in the results' header and
  shared by every source, all with the same corner badges and states:
  **List** (the rows above, for reading titles), **Grid** (even 16:9
  tiles, two to a row, title and line under each) and **Shapes** (each
  tile at its real proportions, rows packed edge to edge, the title over
  the picture's foot and left off a tile too narrow for it). The choice
  is kept per source, so pictures can stay in Shapes and a folder of
  hymns in List.
- **The sources' lists for the phone:** a Bible's books and its chapter
  and verse counts, the clock's presets. The sources have them
  (`sources/bible` counts chapters per translation, `sources/clock` scans
  its presets); the remote needs commands to ask for them.
- **Live on the computer first:** the phone does nothing the dashboard
  can't. The dashboard already lines up (standby) and lists "Shown this
  session" with "Save these as a plan"; its rows still need a click that
  shows the item again, with the ● on the one on stage.
- **Motion** as on the computer: the ring round a step's dot as it goes
  on stage, once; nothing loops.
- **Clicker (to try):** swipe the stage picture left or right for Next and
  Prev, with a tick of vibration, for someone walking the platform.
