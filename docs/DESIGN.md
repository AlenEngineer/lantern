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
| `surface_container_highest` | the progress bar's track |
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

Solid `primary` is kept for the play button and the progress bar of what
plays. A state is never colour alone: the on-stage step has a ●, the next
one a ○, and the chips say "live", "blackout" or "empty".

On stage is `tertiary` (amber with the current palette), not the red of a
broadcast tally, because red is blackout's.

## Separation

Each kind of thing gets the lightest separation that does the job. Try
them in this order, and use a heavier one only when the lighter one
fails:

1. **Space** starts a new group: a section of the plan, a group of the
   library.
2. **Alignment** holds rows of the same kind together. They share
   columns and a height, with no lines between them.
3. **Type** sets the order: size and weight say what to read first.
4. **Brightness** sets focus. What is being worked on sits on the
   brightest surface, and its margins sit darker, with no edge between.
5. **Colour** says what belongs together: a section's colour is on its
   name and on its stretch of the rail, the line down the steps' dots.
6. **Hairline rules** go only where a label and its value read across a
   line, as in the facts under the preview.
7. **A card, a float or a shadow** is only for what sits over the rest:
   the strip, a panel that drops from it, a dialog. A picture of the stage
   has its own edge and needs no card.

## Sections are headers

In every mode a section is a header over its steps: its name in its
colour, its time against its budget, and its stretch of the rail in its
colour. A section inside another sits further in. While planning the
header has a gauge of its time; in a run it has none, and a pace that is
not on track says its word in its tint. A folded section, or one that is
over in a service, is its header alone, its steps one stacked dot. While
a run is locked the plan is the road the service takes, and a section has
a header each time the road comes back to it.

## Planning: the plan is the page

In Plan mode the window has three columns under the strip, in the order
things move. Things come from the library on the left, go into the plan
in the middle, and come out on the stage on the right.

- The plan is the widest column and sits on `surface`, the brightest
  ground. It has no card and no edge.
- The library and the preview are its margins, and they are the window's
  own ground: `surface` darkened, out to the window's edges, with no frame
  round them and no edge between them and the plan. The window has that
  one ground, and the plan is the one surface on it.
- The strip floats on the ground as a pill, apart from the plan.
- With **Glass** on, brightness is how see-through a surface is. The plan
  lets the least of the desktop through.
- A step's row has its clock time, its dot, a 16:9 picture with its size
  in the lower right corner and its state in the upper left, its title,
  where it is from and what it is, and its length.
- The library has two groups, each under its name small, grey and in
  spaced capitals: Lantern's own sources and the steps it always has, then
  your folders. Each is a row, not a card: an icon, a name, and quietly on
  the right what it says, such as which Bible. A source opens the search
  on it. A file is a row as a step is: its picture with its size, its
  name, and a line of what it is, or "○ in the plan" in `primary`.
- Three small icons on the folders' heading lay out the open folder, the
  one in use lit in the `primary` tint: **List**, those rows; **Grid**, even
  16:9 tiles two to a row, each with its name and its line under it; and
  **Shapes**, each tile at its picture's own shape, the rows packed edge
  to edge about as tall as a Grid tile's picture, its name over the
  picture's foot where the tile has room for it, after a ○ in `primary`
  when the plan has it. Every tile has the corner badges, and a picked one
  is in the `primary` tint, or in Shapes ringed in `primary`. Each folder
  keeps its own layout, also for the next start.
- The preview has its name over it as the library's groups do, then the
  picture in its own frame, and under it the slides or pages already
  loaded, small, the one shown ringed in `primary`. Then the name of what
  is looked at, in `primary` when it is a step of the plan, and its facts
  as a table: for a step, where it is from, how it plays and goes on,
  whose look it has and its section; for anything else, where it is from,
  what it is and how long or big. Its actions are pills under the table.
  A step's Look & timing opens its settings under the pills, a group with
  its name over it as the preview's is: where its picture and sound come
  from, its layout, and its fades. Pressed again, or with another step
  picked, it closes. The step's row stays a row; in a run the picked
  step's settings are a card under it.

Run and Live still draw the three cards described below.

## Shape and space

- In Run and Live the window is a ground (`surface`, darkened) with three
  cards on it: the strip, the plan and the stage. A card is `surface` with
  22px corners (the strip is a pill), a hairline of light and a shadow,
  10px from its neighbours and 12px from the window's edge. No lines
  divide a card.
- With **Glass** on, the ground and the cards let the desktop show through,
  the cards less than the ground so text on them reads.
- The strip is a card's colour, with a shadow and no outline. A panel
  that drops from it is the same colour, set off from the card under it by
  its shadow, and meets the strip without a seam, its two shoulders curving
  from the strip's underside into its sides, like a drop leaving the water
  it hangs from. It's as tall as what's in it, up to the window's foot;
  past that its list scrolls.
- Rows and monitors have 12px corners. Buttons and chips are pills.
- Spacing steps by 4px: 4 between buttons, 8–12 inside rows and sections,
  16 around pane content.
- Text is Inter: 16px bold for headings, 13px for body, 12px for
  metadata, 9px bold on a small picture's corner.
- Icons are Material Design Icons, tinted with the text they sit beside.
- While planning, a plan step is a line of title and one of where it is
  from and what it is; in a run it is one line. A result is a line of
  title and one of metadata. What doesn't fit is cut at the row's edge,
  never wrapped.

## Keys

Typing goes to the search box while the search is open, so shortcuts are
keys that don't type (see `crates/app/src/keys.rs`), and each can be
changed in Settings. A button shows its key in its label ("Blackout ·
Ctrl+B"), and while the search is closed the middle pane lists the keys
worth knowing.
