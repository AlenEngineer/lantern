# How the dashboard looks

The dashboard follows a small set of rules so that every pane reads the same
way and so it can take its colours from a desktop theme. The code for each
rule lives in `crates/app/src/look.rs`; this page says what the rules are.

## Colours are roles

Every colour is a Material colour role, read from the `colors` file in the
config (see the README) or from Material's baseline dark palette. Code never
names a hex value outside `palette.rs`.

| Role | Used for |
| --- | --- |
| `surface` | the window |
| `on_surface` | text and icons |
| `on_surface_variant` | secondary text: subtitles, counts, hints |
| `outline` | text for steps already done, empty-monitor notes |
| `outline_variant` | dividers, a field's edge |
| `surface_container` | the search field |
| `surface_container_highest` | the timeline's track |
| `primary` | headings, focus, what is selected or comes next |
| `on_primary` | the play button's icon |
| `tertiary` | what is on stage now |
| `error` | blackout, failures |

## States are tints, not new colours

A state is its role at a fixed strength over the surface, never a different
solid block:

| State | Treatment |
| --- | --- |
| Hover | `on_surface` at 12% over whatever is there |
| Selected, next | `primary` at 22%, bold `primary` text |
| On stage | `tertiary` at 22%, bold `tertiary` text, a ● marker |
| Blackout, destructive | `error` at 14%, `error` text |
| Neutral button | `on_surface` at 8% |

A section's pace uses the same tints: on track is plain, tight is the
`tertiary` warning tint, over is the `error` tint, and each says its word.

Solid `primary` is kept for the play button and the timeline. A state is
never colour alone: the on-stage step has a ●, the next one a ○, and the
chips say "live", "blackout" or "empty".

On stage is `tertiary` (amber with the current palette), not the red of a
broadcast tally, because red is blackout's.

## Shape and space

- The window is a ground (`surface`, darkened) with three cards on it:
  the strip, the plan and the stage. A card is `surface` with 22px corners
  (the strip is a pill), a hairline of light and a shadow, 10px from its
  neighbours and 12px from the window's edge. No lines divide a card.
- With **Glass** on, the ground and the cards let the desktop show through,
  the cards less than the ground so text on them reads. A panel that drops
  from the strip stays nearly solid.
- Rows and monitors have 12px corners. Buttons and chips are pills.
- Spacing steps by 4px: 4 between buttons, 8–12 inside rows and sections,
  16 around pane content.
- Text is Inter: 16px bold for headings, 13px for body, 12px for metadata.
- Icons are Material Design Icons, tinted with the text they sit beside.
- A plan step is one line; a result is a line of title and one of
  metadata. What doesn't fit is cut at the row's edge, never wrapped.

## Keys

Typing goes to the search box while the search is open, so shortcuts are
keys that don't type (see `crates/app/src/keys.rs`), and each can be
changed in Settings. A button shows its key in its label ("Blackout ·
Ctrl+B"), and while the search is closed the middle pane lists the keys
worth knowing.
