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
- **Plans is the plan pane's home.** With no plan to reopen, Lantern starts
  on the list: the plan that's open, templates, **New plan** and your saved
  plans. A new plan starts on its name. **‹** beside the name goes back to
  the list, and **Add step** at the end of the plan replaces the four
  buttons at the bottom.
- Keys can be changed: a **Keys** section in Settings, or `[keys]` in the
  config (`new = "ctrl+t"`). Buttons, tooltips and hints show the keys in
  use.
- The plan pane works from the keyboard. Without a search, ↑ ↓ move
  through the plan's steps, or the saved plans in the list, and Enter shows
  the step or opens the plan. Ctrl+N in a plan opens **Add step** (↑ ↓
  choose the kind, Enter adds it); in the list it's still a new plan. Esc
  leaves the list for the open plan.
- The search opens on a key, Ctrl+K, in the middle pane. Esc empties it and
  then closes it; closed, the middle lists the keys as they're set. Typing
  outside the search no longer goes into it.
- The stage pane shows only once something is on stage, lined up, or coming
  next in the plan.
- Del removes the selected step.
- Steps are edited in their own row. Selecting one no longer opens a strip
  under it: F2 (or the pencil) turns the name into its field, a click on
  the length or a section's time turns that into its field, and the
  selected row carries its own small buttons (go on when done, rename, ↑ ↓
  ✕). Enter or Esc is done. The pink line shows only while a search has a
  result to add, across the plan where it would go.
- **⋯** in the plan's header opens the plan's own settings: its name, its
  look, making it a template, **Duplicate** and **Delete**. It replaces the
  Look button and the step count, so the name has the header's room; "3
  of 7" still shows while the plan runs.
- Settings show one part at a time, picked from a list (This plan, Stage,
  Output, Library, Appearance, Keys), and take the stage pane's room as
  well as the middle's. A key is changed by clicking it and pressing the
  new one; ↺ puts the usual key back.
- The plan and stage panes can be resized: drag the line between two
  panes, or double-click it to let the pane size itself again. The widths
  are remembered. The list of the settings' parts is narrower.
- A plan can have folders of its own (**Its folders** in the plan's
  settings): media only that plan needs, searched while it's open and
  not after. They're saved in the plan.
- A plan can pick its Bibles (**Its Bibles** in the plan's settings): the
  ones it searches, out of those in Settings. All of them until one is
  picked.
- A plan can have a start time (**Starts at** in the plan's settings).
  Sections still to come say when they're due, the header counts down to
  the start, and once the service runs it says how far behind or ahead of
  that schedule it is.
- **Standby**: Alt+Enter keeps the result you're looking at ready under
  the plan's steps without putting it in their order, for a spare song or
  a notice. A click lines it up, a second click or Enter shows it, and the
  plan keeps its place. ↓ past the last step reaches it; ✕ or Del takes
  it off. Saved with the plan.
- The open plan's pictures and PDF pages stay loaded while it's open (up
  to 512 MB of them), so any step or standby item shows at once, in any
  order. Videos are still made ready one step ahead.
- A plan can be taken or sent to another computer as one file. **Export**
  in the plan's settings puts it and its media (the files it shows, its
  own folders, its look's image, its downloaded YouTube videos) into
  `<plan>.lantern` in Documents, a zip; **Import a plan** in the list of
  plans finds exported plans there, in Downloads and on what's plugged in,
  copies the media in and opens the plan.
- **Glass**: Settings → Appearance lets the desktop show through the
  dashboard, from solid to 80%, with a switch for blurring what shows
  through. The stage stays solid.
- **A new layout.** A strip across the top holds the plan's name, the
  running section's time, how the service stands against its schedule, the
  time of day, Search, what isn't ready, the stage's screen, Blackout and
  Settings. The plan is on the left and the stage in the middle: what the
  audience sees, large, with what comes next beside it. Ctrl+K drops the
  search from the strip, with the result you're looking at under the list.
  The list of keys in the middle is gone (they're under Settings → Keys).
- A plan step is one line: its mark, its name, and at its end the one
  thing worth saying of it (what's wrong, how far the stage is into it,
  its kind, or its length). The selected step opens a second line with its
  buttons. Steps and standby items no longer carry a thumbnail.
- A YouTube video shown outside the plan plays straight from YouTube and
  leaves nothing on disk; the plan's are still downloaded ahead of time.
  It can be paused, scrubbed and replayed, and is downloaded instead if it
  can't be streamed. Settings → Library → YouTube turns it off.
- **Time on stage**: `!t` finds a clock, a timer that counts up, and
  countdowns (`!t 7` for seven minutes, `!t 10:30` to count down to half
  past ten). They're shown, planned and kept on standby like anything
  else, in large figures.
- While a Bible passage is on stage, its lower right corner says the
  verse it's at and the Bible ("Juan 3:17 · RVG"). Settings → Stage →
  Verse hides it.
- `!x` shows what you type after it: `!x Bienvenidos`. ` | ` starts a
  new line and ` || ` a new slide.
- The strip has a button for each source: the Bible, YouTube, folders,
  images, time and your own text, and a folder's own bang as `!h`. A
  button opens the search on that source, as typing its bang would.
- Live YouTube videos play. A video that is live now plays as it comes, at
  720p, off the plan or in it: its step says `live` where a download would
  show, and the time under it says `live`. It can't be scrubbed, and it
  isn't downloaded. Live videos used to fail with "Requested format is not
  available".
- **A remote on your phone.** Settings › Phone turns it on and shows a QR
  code; the phone's camera opens a page, with nothing to install. It shows
  the plan, what is on stage and what is next, the section's time and how
  the service stands, and it has Next and Previous, play or pause, the
  time bar, Blackout, Clear, and any step of the plan two taps away. The
  phone and the computer must be on the same network. Off until you turn
  it on; `remote_port` in the config changes its port from 7373.
- The phone remote searches. The magnifier opens a search of everything
  Lantern has: type a verse, a hymn, a file's name or a link, or start
  from a button for one kind of thing (`!b`, `!yt`, `!x` for text you
  type). Tap a result, then **Show now** or **Add to plan**. It is the
  phone's own search: nothing appears in the dashboard's box.
- The phone remote shows the stage. The eye puts a picture of what the
  audience sees above the title, new about once a second, video frames
  included. Lantern makes the picture only while a phone is showing it.
- A folder can have a **names file**, for files named by number or code: a
  line for each, its name, a tab, its title (`004<tab>Himno 4 · Alabanzas
  sin cesar`). Set it under the folder's path in Settings, or with
  `names = "…"` in the config, and the files are shown and found by
  their titles.
- **PowerPoint files show as slides.** A `.pptx`, `.ppt`, `.ppsx`, `.pps`
  or `.odp` is stepped through like a PDF, with its first slide as its
  thumbnail. It needs LibreOffice, which turns the file into pages the
  first time it is shown; animations and transitions don't play.
- **A deck's videos.** When the slide on stage has a video kept in the
  `.pptx`, a YouTube video or a link to a picture, a button under the
  slides shows it. Next then goes on to the following slide, and Prev back
  to the one it was on.
- The phone remote shows a deck's videos: the slide on stage lists what it
  plays or links to, a button each, and **Back to slide N** returns from
  it. Next and the back arrow do as they do on the computer.
- **Sound files play.** An `.mp3`, `.wav` or other sound file goes on
  stage like anything else, with a timeline, and what was on stage stays up
  while it plays: slides, a passage, a picture. A sound kept in a `.pptx`
  gets a button on its slide.
- Slides go on turning under a sound: with a sound playing over a deck, a
  PDF or a passage, Next and Prev step its slides, and the plan moves on
  only after the last one. The slide strip stays, and the sound's line
  says what it plays over.
- **A step can bring its own sound**: music under a passage, or under the
  last slides of a deck. Select the step, pick the sound in search, click
  the note on the step's row, and set the slide it starts at. It plays over
  the step's slides as they turn and stops with the step.
- **Three modes: Plan, Run, Live**, switched at the top. Planning never
  reaches the projector. A running plan is locked, and **Unlock** (Ctrl+E)
  lets what is still ahead be changed until the service moves on. Live has
  no plan, lists what was shown, and can save it as one.
- **The plan pane is a tree while planning**: the open plan, its sections
  (an arrow folds one away) and their steps, and under them the other plans
  and the templates, a click from being opened in its place.
- A step shows what is inside it, under its row: the videos, sounds and
  pictures on a deck's slides, by slide, and the sound that plays with the
  step. In a run, the slide the service is on is lit.
- **The library is a tree beside the plan** while planning: the media
  folders and their files. A click looks at one on this screen; a double
  click or Enter adds it to the plan after the selected step.
- **Replace a step with what is picked.** With a step selected and a
  search result or a library file picked, a button puts the pick in the
  step's place: it keeps its position and its sound, and takes the new
  name unless you had named it yourself.
- The lines inside a step go to their slide: in a run the slide goes on
  the projector, and while planning it is looked at in the preview.
- The library's folders nest as they do on disk, and a **filter** box at
  its top narrows it to the files whose name or folder has what you type.
  A folder shows all its files, not the first 200.
- **Drag a file from the library into the plan.** A gap opens where it
  will land, above or below the step under the pointer.
- **A service cut short is picked up.** Closed or crashed mid-service,
  Lantern comes back in Run with the plan waiting at the step and slide it
  was at; nothing is shown until **Pick up there** (PgDn).
- A step under the pointer offers what can be done to it: **✕** takes it
  out, and with something else picked, **Replace** puts that in its place.
  Tips wait a moment before they show, so passing over a button shows
  nothing.
- The step that goes on the projector rings once: its dot glows and its
  row is lit a shade more, then both settle. When the phone moved the
  stage, the stage says **from the phone** for two seconds, so whoever is
  at the keyboard knows it wasn't them. **Motion** in Settings (or
  `no_motion = true`) turns the moving off.
- A refusal can be seen. When a mode won't do what was asked (an edit to
  a locked run, something for the projector while planning), the mode's
  note says why, lit, and shakes once. Pulling at a step the run won't let
  move does the same, under a "not allowed" pointer.
- A step that was just added, replaced or dropped in its new place
  flashes once, so the eye finds where it went.
- The mode switch's lit pill slides to the mode and takes its colour. A
  section's steps close under its heading and open again, and the gap
  where something dragged would land opens instead of appearing.
- A section's heading under the pointer shows **✕** too. Every button
  with a key says it when the pointer rests there: Search, Blackout,
  Settings, Save, Clear the stage and play/pause now do, like the rest.
- A file outside the media folders can go in a plan: paste or type its
  path in the search, or paste a file copied in the file manager. The
  step then offers **keep a copy**, which copies the file to where Lantern
  keeps plans' media so the plan doesn't depend on the original.
- **Space goes on.** With the search closed, Space is next: the big key
  for the thing done most. In an open search it still types, and right
  after letters typed into a closed search it is let go, so a search typed
  by mistake never moves the service.
- Ctrl+Shift+B clears the stage. It had a button and no key.
- The modes have keys: Ctrl+1 is Plan, Ctrl+2 starts the service, Ctrl+3 is
  Live. In a run, Ctrl+1 pressed twice ends the service. A key that can't
  go where it points says why, as the buttons do.
- The trees work from the keyboard. While planning, → opens a section and
  then steps into it, ← closes it, and from a step goes up to its heading;
  ↑ ↓ pass over a folded section's steps. Home and End go to the first and
  last row. F6 gives the arrows to the library, where the same keys walk
  its folders and files and Enter adds the file to the plan. In a run and
  in Live, → ← are still next and back.
- Ctrl+Shift+Z does again what Ctrl+Z took back. A new edit of the plan
  ends what can be redone.
- Buttons no longer carry their keys in their labels: Unlock, New plan,
  Add to the plan, Show now, and Save and Close in Settings say their key
  once the pointer rests on them, like the rest. A key stays in sight
  where the words are the instruction ("Ctrl+Enter adds here"), and on
  **Next step**, which now names Space.
- Buttons no longer carry their keys in their labels: Unlock, New plan,
  Add to the plan, Show now, and Save and Close in Settings say their key
  once the pointer rests on them, like the rest. A key stays in sight
  where the words are the instruction ("Ctrl+Enter adds here"), and on
  **Next step**, which now names Space.
- **The stage opens when it is needed.** Lantern starts with the dashboard
  alone; the stage window opens with **Start the service** or in Live and
  closes on the way back to Plan. A screen button in the strip, and F7,
  open or close it by hand until the mode changes. Closing the stage
  window no longer closes Lantern.
- Closing the dashboard while the projector is in use asks first: it has
  to be closed twice within four seconds. With a service or Live going and
  the stage window closed, its button is red and the live picture says
  "the stage is closed".
- Less standing text. The line over the plan pane no longer describes the
  mode: it shows only when something is refused, and in a run it holds
  Unlock. The preview is headed just "Preview", the empty preview and the
  empty Live list say nothing, and what each Settings section is about
  shows when the pointer rests on its title.
- A video on the projector plays in the dashboard's live picture too,
  not as a still: it is the same player, drawn twice. With the stage
  window closed the video still plays there and its end is still noticed.
- **The stage knows where to go.** A preferred screen is kept by its make
  and model, and while it is connected the stage is open on it, from the
  moment it is plugged in. Otherwise the stage opens with a service on a
  screen the dashboard isn't on, and a stage moved by hand is remembered
  for that set of screens. With one screen there is no stage window: the
  live picture is the stage. A stage whose screen is unplugged mid-service
  closes, and the live picture says the projector is gone.
- The screens stay on while it shows. With a service running or anything
  on the projector, Lantern asks the desktop not to idle, and lets go when
  the service ends and the stage is clear.
- Notifications wait for the length of a service, on desktops whose
  notification service can be asked (`Inhibit`, as in KDE). For the
  others, `on_service_start` and `on_service_end` in the config name a
  command to run as a service starts and as it ends.
- A look round as a service starts. If a step of the plan won't show, the
  sound is muted or at 0, or sound goes to a different output than last
  service, **Start the service** says so first, with **Start anyway** and
  **Not yet**. With nothing to say, the service starts as before.
- The look round remembers where sound went for each set of connected
  screens. Rehearsing at home on the speakers no longer makes it warn at
  church, where the projector is plugged in, or the other way round.
- Sections inside sections, three levels deep. **Alt+→** puts the selected
  section inside the one before it and **Alt+←** moves it out a level; the
  same two arrows are on a selected section's row. A section holds the
  sections in it: folding, dragging and its time count them all.
- A section or a step can change the look it is given: a `look` on it in
  the plan file sets only what it names (background, image, text colour,
  font, sizes, dim, the reference line), and the rest carries down from
  the plan and the sections around it. There is no panel for it yet; the
  dashboard and the stage already follow it.
- Flow, in the plan file for now. A step, a section or the plan can
  repeat (`"flow": {"repeat": {"times": 2}}`, `"forever"`, or
  `{"until": "10:30"}`) and a section or the plan can shuffle its steps,
  drawn afresh each round. Going on by itself carries down: set on a
  section or the plan, its steps do it unless one says otherwise. Space
  follows the flow, innermost repeat first, and Up next shows what will
  really come.
- A rail down the left of the plan's steps shows how it moves: a filled
  dot joined to the next step goes on by itself, a hollow one waits. A click
  under a dot joins or parts two steps, in place of the **auto** switch.
  In a service the rail is grey where it has been, the stage's colour
  where it is (the line fills as the step plays), and ringed at what is
  next. Step numbers and the ↦ mark are gone.
- Flow has its buttons. A section, and the plan on its row at the top,
  shows three: its steps go straight on or wait, it repeats (once, ×N, ∞ or
  until a time), its steps are shuffled. A step has repeat among its
  actions and wears a mark when it comes again.
- How long the plan takes, large, over the plan: the total, a target for
  the whole service (◎) with how far over or under it is, what the steps
  themselves add up to, when it ends, and a bar with a part for each
  section. What repeats a number of times counts that often, in the total
  and in each section's time.
- A panel beside the preview with three lenses. **+** is the library, as
  before. The sliders edit the look of what is selected (a step, a section
  for everything in it, or the plan), each value saying which level it
  comes from, with **set here ✕** to give it back. The tick shows what
  isn't ready, each section against its time, and where sound and the
  stage go.
- Rehearsal. ▶ on the plan's row or a selected section walks it through
  in the preview as the service would take it, following its flow, with
  nothing on the projector: pause, on to the next (Space), a speed for its
  clock, stop, and at the end how long it took.
- The plan pane redrawn as an outline. A section is one line: name, a bar,
  `9/15`, how it moves, ▶. The time row leads with what is planned so far
  against the target, with what is left under it. A selected row stays one
  line and shows its actions at its end under the pointer. The preview and
  the side panel are a card each, the lenses one switch, and the preview
  wears a mark for "not on the projector". Gone from the plan view: the
  plan's title row, "unsaved", the other plans and templates under it,
  the standing fold arrows and the words "Add step".
- How often something comes is chosen on a card of its own, at the top of
  the side panel: a switch with four places (once, ×, ∞, until a time), a
  stepper for the number with what it comes to ("5:15 × 2 = 10:30"), or a
  field for the time. A selected section's row keeps still under the
  pointer, with its actions on a second line.

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
- Ctrl+N, or opening another plan, asks first while the plan is running;
  one key mid-service took the plan away.
- The Next button says "Next slide · PgDn" while PgDn steps through what's
  on stage, and works then even at the end of the plan.
- With something shown on the fly in the middle of the plan, On stage says
  "off the plan", the step says it's waiting, and **Back to the plan**
  shows it again.
- Downloaded videos no longer say "ready" on every row and caption; only
  queued, downloading and failed are said.
- "Ctrl+A adds the result here" shows only while a search has a result to
  add (the pink line stays), and the line of keys under the results only
  with the search empty before the plan runs.
- The plans list's headings no longer have a line of instructions under
  them.
- The home screen (the middle, with the search closed) leads with a search
  box to click and sets the keys in three columns, where it had one long
  column and a scrollbar in the middle of the pane.
- Importing a plan when one of the same name is already saved names the
  new one apart ("Domingo (imported)"), and says it's the imported one
  that opened. They were two rows that read the same.
- Opening another plan (or deleting the open one) no longer changes the
  font and colours of what's on stage. It keeps the look it went up with;
  the new plan's look starts with the next thing shown.
- Ctrl+A in the search box only selects what's typed. It also added the
  result to the plan, unnoticed. Adding is now Ctrl+Enter, and standby
  Alt+Enter; both can be changed under Keys.
- "Behind" and "ends in" count the section under way for its whole
  time, as they do the ones to come. A service that started late read as
  ahead while its section had time to spare.
- **Up next** says "No plan open" or "This plan has no steps yet" when
  that's why nothing is next. It said "The end of the plan" either way.
- A folder that isn't on this computer says so under its row in
  Settings, for the plan's own and the library's, and the save notice
  names it. It was saved without a word and never searched.
- Settings keep what's on stage in sight: while something is live, their
  header shows its state, its name and a small picture of it. They cover
  the stage pane, which left the operator blind mid-service.
- Settings say when they have unsaved changes, and Esc asks once before
  discarding them: the first press says so, the second closes. Esc
  dropped them without a word.
- Two actions on one key show at once under Keys, in red on both rows
  with the other's name. Only Save said so. ↺ says which key it puts back.
- The Blackout button is solid red while the projector is black and says
  "Blackout is ON: end it". On and off looked almost the same.
- **Its Bibles** says in words what the plan searches ("Searching only
  RV1909"), and with a single Bible says so in place of a toggle that
  changed nothing. It names a Bible the plan asks for that isn't here.
- An example in an empty field is dimmer and starts with "e.g.", so it
  isn't taken for a value. The import field's line reads "Or type where
  the file is".
- Ctrl+Z right after deleting a plan brings it back, and the notice says
  so. It did nothing.
- The line under the plan (an export's result, an error) wraps where it
  was cut off, and a selected step's length looks like the button it is.
- In a narrow plan pane a section keeps its name, and its time is what
  gets shorter ("8 min / 15 min") or cut. The name was cut first.
- The search drops from the strip as one body with it: the same colour, no
  line where they meet, and curved shoulders into the strip. It's as tall
  as what it holds, and its list scrolls once it reaches the window's foot.
  It was a full-height box of another colour.
- Esc empties the search and keeps its bang chip, so the next search asks
  the same source: several verses or videos in a row. Esc on an empty box
  closes the search; Backspace there still takes the chip away.
- The strip and the search that drops from it are the cards' colour; a
  shadow sets the dropped search off from the card under it.
- A YouTube video with surround sound plays straight from YouTube like any
  other, with its stereo track; it used to fail to start and be downloaded
  instead. A stream on a slow network gets twenty seconds to start, not
  five, before Lantern falls back to downloading it.
- A video that stops on an error says so. A stream whose network went away
  used to look like a video that had reached its end, and a step set to go
  on by itself moved on; now the stage reports it failed, with the reason,
  and the plan stays where it is.
- The time under a video stays where you dragged it while the video gets
  there. It used to fall to 0:00 until the picture came back, which on a
  stream is several seconds.
- Jumping around in a video streamed from YouTube is quick: about a second
  for a short jump and a few for a long one, where a jump of some minutes
  took up to twenty. Lantern now reads the stream in pieces, the way
  `yt-dlp` downloads, which YouTube doesn't slow down.
- A video YouTube refuses for its first moments still plays straight from
  YouTube. About half the time a stream's address is refused for a second
  or two after it is issued; Lantern used to give up at once and download
  the video instead, and now asks again for up to ten seconds. When a
  stream can't start, the log says what YouTube answered.
- A video streamed from YouTube is heard. The sound server remembers when
  Lantern's sound was last muted, which a video loading ahead does to
  itself, and a stream could start under that mute and play in silence.
- A jump in a long video streamed from YouTube lands in a second or two.
  An hour's jump in a two-hour video left the picture still for over a
  minute, long enough to look frozen. The stream's files each begin with
  an index, and Lantern now plays them in a way that uses it. A stream
  whose network goes away is still reported as failed, not ended.
- Going down a list of YouTube results no longer asks YouTube for every
  result on the way. A result has to stay lined up for a moment before
  Lantern fetches it; each one passed used to start a `yt-dlp` of several
  seconds that nothing stopped.
- The phone remote answers a browser that connects first and asks later.
  Phone browsers open connections ahead of time; Lantern closed one that
  waited ten seconds, and the browser gave up on the page ("too many
  retries"). Settings › Phone now also says whether a phone has reached
  the computer, which tells a problem of the network's from one of
  Lantern's.
- The phone remote's address and code follow the network. After the
  computer joined another Wi-Fi or a hotspot, Settings › Phone went on
  showing the address it had when the remote started.
- A video taken while it was still loading starts by itself: double-clicking
  a file showed it held at 0:00 until it was paused and played again.
- A number finds the file numbered so: `4` finds `004.mp4` first, then
  `040`, `041`, `400`, where it used to list `400` to `499` and never
  `004`. Results that match equally come in number order, not `1, 10,
  100, 2`.
- While a slide's video plays, the Next button says what it does, **Next
  slide** or **Back to slide N** after the deck's last slide, where it read
  "Next step" and looked switched off though the key worked.
- Dragging a step: the rows no longer jump. The gap starts where the step
  was picked up, and as it moves the one it left closes while the new one
  opens, so the rows between slide over. Let go where it was picked up,
  nothing moves and nothing is added to undo.
- Three small jumps in the dashboard. A refusal outside a run ("Nothing
  reaches the projector while planning") is said over the top of the pane
  and no longer pushes the plan down and back. The mode switch keeps the
  mode it is going to in sight while its light slides there. A row no
  longer grows under the pointer and nudges the rows below it.
- The mode switch shows Run and Live again while planning: the fix above
  had cut it down to the lit pill.

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
