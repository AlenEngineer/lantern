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
- App, later: show the slide strip of what's on stage (verse numbers), and
  click one to jump to it.

## Plan

- **Live slots** carry a search (`!yt`, `!b`, a folder's bang); reaching one
  opens the search ready, and what was shown is kept in the slot. *(done)*
- **Progress** in each plan row: slide or page of what's on stage. *(done)*
- **Ready check** before the service: downloads done, files present, kinds
  the stage can show, image links downloaded. *(done; reading every PDF
  ahead is still to do)*
- **Templates:** start a plan from a saved order of service.
