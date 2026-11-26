//! Scripted mode: drives the engine from a file, one command per line, so
//! `scripts/e2e.sh` can run the real app and screenshot it.
//!
//! To add a command, add a variant to [`Command`], a line to
//! [`Command::parse`] and an arm to [`Script::perform`].

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lantern_engine::{
    Engine, Hit, Kind, Location, Query, Resource, ResourceId, SearchEvent, SourceId,
};
use lantern_source_link::LinkSource;

use crate::drag::Half;
use crate::keys::{self, Op};
use crate::library::Library;
use crate::log;
use crate::youtube::{Downloads, YoutubeVideo};

/// What `show`, `preview` and `add` put on stage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// A local file. A relative path is relative to the script's folder.
    File(PathBuf),
    /// A link, resolved by the link source as if pasted into the search box.
    Link(String),
}

impl Target {
    fn parse(text: &str, dir: &Path) -> Result<Target, String> {
        if text.starts_with("http://") || text.starts_with("https://") {
            links()
                .resolve(text)
                .ok_or_else(|| format!("not a link: {text:?}"))?;
            return Ok(Target::Link(text.to_string()));
        }
        Ok(Target::File(dir.join(text)))
    }

    /// The resource the app would have for it: a file in one of the
    /// library's folders as the library has it (its folder, its title,
    /// its size), any other as its pasted path would be, and one that
    /// isn't there by its path alone, to be missing.
    ///
    /// Starts downloading a YouTube link. The app does too, but only once
    /// its event loop catches up, and a following `wait-ready` must see it.
    fn resource(&self, engine: &Engine, library: &Library, downloads: &Arc<Downloads>) -> Resource {
        let resource = match self {
            Target::File(path) => library_file(engine, library, path)
                .or_else(|| links().resolve(&path.to_string_lossy()))
                .unwrap_or_else(|| file(path)),
            Target::Link(url) => links().resolve(url).expect("checked by Target::parse"),
        };
        if let Some(video) = YoutubeVideo::of(&resource) {
            downloads.fetch(&video);
        }
        resource
    }
}

fn links() -> LinkSource {
    LinkSource::new(SourceId::new("link").expect("a valid source id"))
}

/// One line of a script.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Command {
    /// `wait MS`: sleep.
    Wait(Duration),
    /// `hymn`: add the built-in hymn to the plan and go to it.
    Hymn,
    /// `text "..."`: show inline text now, off-plan. `\n` starts a new line.
    Text(String),
    /// `next`: the next button.
    Next,
    /// `prev`: the back button.
    Prev,
    /// `black`: flip blackout.
    Black,
    /// `clear`: empty the stage.
    Clear,
    /// `show PATH|URL`: show a file or link now, off-plan.
    Show(Target),
    /// `preview PATH|URL`: line a file or link up in preview.
    Preview(Target),
    /// `add PATH|URL`: add a file or link to the plan after the current step.
    Add(Target),
    /// `section BUDGET TITLE`: add a section with a budget (`15`, `0:20`,
    /// or `none`) after the current step.
    Section(Option<Duration>, String),
    /// `slot SEARCH`: add a live slot that searches SEARCH (`!b jn 3`).
    Slot(String),
    /// `path TEXT`: type TEXT in the open settings' first folder path,
    /// adding a folder row if there's none.
    Path(String),
    /// `check-key`: open the settings at Library and check the YouTube
    /// key, as its button does.
    CheckKey,
    /// `pick-on-youtube`: open the picker on what the search box holds,
    /// as its button does.
    PickOnYoutube,
    /// `picked ID`: a video picked on YouTube's site, as a click there
    /// tells it.
    Picked(String),
    /// `font TEXT`: type TEXT in the open settings' font field, scrolled
    /// into view.
    Font(String),
    /// `wait-ready SECS`: wait until no YouTube download is queued or running.
    WaitReady(Duration),
    /// `take`: put the preview on stage.
    Take,
    /// `play`: play the program.
    Play,
    /// `pause`: pause the program.
    Pause,
    /// `seek SECS`: jump the program's timeline.
    Seek(Duration),
    /// `frame N`: go to page or stanza N, counted from 1.
    Frame(usize),
    /// `search TEXT`: wait for the index, then search it and the live
    /// sources, logging the hits.
    Search(String),
    /// `show-hit N`: show hit N of the last search now, counted from 1.
    ShowHit(usize),
    /// `type TEXT`: wait for the index, then put TEXT in the dashboard's
    /// search box, as if typed.
    Type(String),
    /// `select N`: pick result N in the dashboard, counted from 1, which
    /// previews it. Web pages are skipped, as on screen.
    Select(usize),
    /// `click N`: click result N in the dashboard, counted from 1. Twice in
    /// a row is a double click, which shows it.
    Click(usize),
    /// `step N`: click entry N of the plan, counted from 1, sections
    /// included: it becomes the selection. Twice in a row is a double
    /// click, which shows it.
    Step(usize),
    /// `fold N`: fold the section at plan entry N, counted from 1 as for
    /// `step`, into its header, or open it again.
    Fold(usize),
    /// `library NAME`: click the library's row named NAME (`Bible`,
    /// `Live`, `Fixtures`, a file's title): a source opens the search on
    /// it, a step is added, a folder opens or closes, a file is looked at.
    Library(String),
    /// `library-layout list|grid|shapes`: lay out the library's open
    /// folder so, as the icons on its heading do.
    LibraryLayout(crate::shelf::Layout),
    /// `drag N M above|below`: press on plan entry N and hold it over the
    /// upper or lower half of entry M, as the mouse would, counted from 1.
    /// `drop` lets go.
    Drag(usize, usize, Half),
    /// `length N TEXT`: type TEXT in plan entry N's length field (`0:03`).
    Length(usize, String),
    /// `go-on N`: join plan entry N to the next, so it goes on when done.
    GoOn(usize),
    /// `fade N IN OUT`: plan entry N fades in over IN seconds and out
    /// over OUT; 0 is no fade.
    Fade(usize, u64, u64),
    /// `loop N M` or `loop N M until 10:30`: draw a loop round plan
    /// entries N to M, to come twice or until the time of day.
    Loop(usize, usize, Option<lantern_engine::TimeOfDay>),
    /// `starts 19:00`: the plan starts at that time of day.
    Starts(lantern_engine::TimeOfDay),
    /// `plan-look COLOUR`: open the settings on the plan's look (the
    /// header's swatch), give it its own and type COLOUR as its
    /// background. `press ctrl+s` saves it.
    PlanLook(String),
    /// `output NAME`: pick the screen NAME for the stage in the settings
    /// (`automatic` leaves it to the compositor).
    Output(String),
    /// `slide N`: click slide N (from 1) in the strip of what's on stage.
    Slide(usize),
    /// `film N`: click slide or page N (from 1) in the filmstrip under the
    /// preview while planning, which shows it there.
    Film(usize),
    /// `template NAME`: star the saved plan NAME (title or file name), so
    /// it's a template.
    Template(String),
    /// `from-template NAME`: click the template NAME: a new plan from it.
    FromTemplate(String),
    /// `drop`: let go of what `drag` holds, moving it to where it's over.
    Drop,
    /// Changes mode: `plan`, `live`, or `start` and `end` for a run.
    Mode(String),
    /// `press KEY`: a dashboard key, as if pressed with the search box empty
    /// (see [`keys::op_named`] for the names). `press ctrl+s` saves
    /// the plan, `ctrl+o` lists saved plans, `ctrl+n` starts a new one.
    Press(String),
    /// `open NAME`: open the saved plan with this title or file name, as if
    /// picked from the open list.
    Open(String),
    /// `rename TEXT`: rename the plan, as if typed into its name.
    Rename(String),
    /// `shot NAME`: ask the harness to capture both windows.
    Shot(String),
    /// A real pointer, moved and pressed through sway: see [`Pointer`].
    Pointer(Pointer),
    /// `quit`: stop here.
    Quit,
}

/// What a real pointer does: the mouse `scripts/e2e.sh` plugs into the
/// sway it runs the app in (`scripts/e2e/mouse.py`), moved and pressed as
/// a hand would, a little at a time. Anywhere else a script with one of
/// these fails before any window opens. Places are in the dashboard's own
/// logical pixels, from its top left corner: a screenshot's pixels
/// divided by `E2E_DASHBOARD_SCALE`, wherever sway has put its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pointer {
    /// `pointer X Y`: moves there.
    To(At),
    /// `pointer-at-row N [top|middle|bottom|foot|chevron]`: moves to a
    /// part of plan entry N, counted from 1, sections included; `middle`
    /// when not said. `pointer-at title`, `pointer-at ring-note` and
    /// `pointer-at library N` move to the other places of [`Aim`]. Each
    /// is where the dashboard last laid it out (a row folded away keeps
    /// its last place).
    Toward(Aim),
    /// `pointer-press [left|right] [ctrl|shift]`: pressed, and held;
    /// `left` when not said. A key said with it is held until
    /// `pointer-release`, as the app sees it, since sway can't press one:
    /// the app is told, as if it had been.
    Press(Button, Option<Held>),
    /// `pointer-release [left|right]`: let go, and of the key held with
    /// the press.
    Release(Button),
    /// `click-at X Y [left|right] [ctrl|shift]`: moves there, presses and
    /// lets go, as `pointer-press` and `pointer-release` do.
    Click(At, Button, Option<Held>),
    /// `drag-at X1 Y1 X2 Y2`: pressed at the first place, carried to the
    /// second, and let go.
    Drag(At, At),
}

/// A place on the dashboard, in its logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct At {
    pub x: u16,
    pub y: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Left,
    Right,
}

/// A key held for one click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Held {
    Ctrl,
    Shift,
}

/// Where a pointer command goes on the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aim {
    /// A part of plan entry N, counted from 0.
    Row(usize, Spot),
    /// The plan's title over its rows. It is no row: a press there lets
    /// go of what is picked.
    Title,
    /// The note's button of the ring that is open.
    RingNote,
    /// Row N of the library in sight, folders included, counted from 0.
    Library(usize),
}

/// Where on a plan entry's row `pointer-at-row` goes, across its middle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spot {
    /// 4 pixels in from its top: a header's head, to pull; a step's upper
    /// half.
    Top,
    Middle,
    /// Three quarters down: its lower half, where what is dropped goes
    /// after it, or into a section first.
    Bottom,
    /// A section's foot: the strip under its last row, to pull.
    Foot,
    /// The arrow on a section's header that folds it, shown while the
    /// pointer is on the header.
    Chevron,
}

impl Pointer {
    fn parse(word: &str, rest: &str) -> Result<Pointer, String> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let counted = |n: &str| n.parse::<usize>().ok().filter(|n| *n > 0).map(|n| n - 1);
        let at = |x: &str, y: &str| {
            Some(At {
                x: x.parse().ok()?,
                y: y.parse().ok()?,
            })
        };
        // `[left|right] [ctrl|shift]`, in either order.
        let pressed = |words: &[&str]| {
            let (mut button, mut held) = (None, None);
            for word in words {
                match *word {
                    "left" if button.is_none() => button = Some(Button::Left),
                    "right" if button.is_none() => button = Some(Button::Right),
                    "ctrl" if held.is_none() => held = Some(Held::Ctrl),
                    "shift" if held.is_none() => held = Some(Held::Shift),
                    _ => return None,
                }
            }
            Some((button.unwrap_or(Button::Left), held))
        };
        let parsed = match (word, &words[..]) {
            ("pointer", [x, y]) => at(x, y).map(Pointer::To),
            ("pointer-at-row", [n, spot @ ..]) if spot.len() <= 1 => {
                let spot = match spot.first().copied() {
                    None | Some("middle") => Some(Spot::Middle),
                    Some("top") => Some(Spot::Top),
                    Some("bottom") => Some(Spot::Bottom),
                    Some("foot") => Some(Spot::Foot),
                    Some("chevron") => Some(Spot::Chevron),
                    _ => None,
                };
                counted(n)
                    .zip(spot)
                    .map(|(n, spot)| Pointer::Toward(Aim::Row(n, spot)))
            }
            ("pointer-at", ["title"]) => Some(Pointer::Toward(Aim::Title)),
            ("pointer-at", ["ring-note"]) => Some(Pointer::Toward(Aim::RingNote)),
            ("pointer-at", ["library", n]) => counted(n).map(|n| Pointer::Toward(Aim::Library(n))),
            ("pointer-press", more) => {
                pressed(more).map(|(button, held)| Pointer::Press(button, held))
            }
            ("pointer-release", more) => match pressed(more) {
                Some((button, None)) => Some(Pointer::Release(button)),
                _ => None,
            },
            ("click-at", [x, y, more @ ..]) => at(x, y)
                .zip(pressed(more))
                .map(|(at, (button, held))| Pointer::Click(at, button, held)),
            ("drag-at", [x1, y1, x2, y2]) => at(x1, y1)
                .zip(at(x2, y2))
                .map(|(from, to)| Pointer::Drag(from, to)),
            _ => None,
        };
        let usage = match word {
            "pointer" => "X Y",
            "pointer-at-row" => "an entry from 1, then top, middle, bottom, foot or chevron",
            "pointer-at" => "title, ring-note, or library and a row from 1",
            "pointer-press" => "left or right, ctrl or shift",
            "click-at" => "X Y, then left or right, ctrl or shift",
            "drag-at" => "X1 Y1 X2 Y2",
            _ => "left or right",
        };
        parsed.ok_or_else(|| format!("{word} needs {usage}, got {rest:?}"))
    }
}

/// A line the parser rejected.
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError {
    line: usize,
    message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl Command {
    fn parse(line: &str, dir: &Path) -> Result<Command, String> {
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let command = match word {
            "wait" => {
                let ms = rest
                    .parse()
                    .map_err(|_| format!("wait needs milliseconds, got {rest:?}"))?;
                Command::Wait(Duration::from_millis(ms))
            }
            "hymn" => Command::Hymn,
            "text" => {
                let body = rest
                    .strip_prefix('"')
                    .and_then(|r| r.strip_suffix('"'))
                    .ok_or_else(|| format!("text needs a quoted string, got {rest:?}"))?;
                Command::Text(body.replace("\\n", "\n"))
            }
            "show" | "preview" | "add" if rest.is_empty() => {
                return Err(format!("{word} needs a path or link"));
            }
            "show" => Command::Show(Target::parse(rest, dir)?),
            "preview" => Command::Preview(Target::parse(rest, dir)?),
            "add" => Command::Add(Target::parse(rest, dir)?),
            "slot" if !rest.is_empty() => Command::Slot(rest.to_string()),
            "path" => Command::Path(rest.to_string()),
            "check-key" => Command::CheckKey,
            "pick-on-youtube" => Command::PickOnYoutube,
            "picked" if lantern_web_protocol::is_video_id(rest) => {
                Command::Picked(rest.to_string())
            }
            "picked" => return Err(format!("picked needs a video id, got {rest:?}")),
            "font" => Command::Font(rest.to_string()),
            "slot" => return Err("slot needs what it searches".into()),
            "section" => {
                let usage = || format!("section needs a budget and a title, got {rest:?}");
                let (budget, title) = rest.split_once(' ').ok_or_else(usage)?;
                let budget = match budget {
                    "none" => None,
                    budget => Some(crate::length::parse(budget).flatten().ok_or_else(usage)?),
                };
                Command::Section(budget, title.trim().to_string())
            }
            "wait-ready" => {
                let secs = rest
                    .parse()
                    .map_err(|_| format!("wait-ready needs seconds, got {rest:?}"))?;
                Command::WaitReady(Duration::from_secs(secs))
            }
            "take" => Command::Take,
            "play" => Command::Play,
            "pause" => Command::Pause,
            "seek" => {
                let secs: f64 = rest
                    .parse()
                    .map_err(|_| format!("seek needs seconds, got {rest:?}"))?;
                Command::Seek(
                    Duration::try_from_secs_f64(secs)
                        .map_err(|_| format!("seek needs seconds, got {rest:?}"))?,
                )
            }
            "frame" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Frame(n - 1),
                _ => return Err(format!("frame needs a number from 1, got {rest:?}")),
            },
            "search" if !rest.is_empty() => Command::Search(rest.to_string()),
            "search" => return Err("search needs text".into()),
            "show-hit" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::ShowHit(n - 1),
                _ => return Err(format!("show-hit needs a number from 1, got {rest:?}")),
            },
            "type" if !rest.is_empty() => Command::Type(rest.to_string()),
            "type" => return Err("type needs text".into()),
            "open" | "rename" if rest.is_empty() => return Err(format!("{word} needs a name")),
            "open" => Command::Open(rest.to_string()),
            "rename" => Command::Rename(rest.to_string()),
            "step" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Step(n - 1),
                _ => return Err(format!("step needs a number from 1, got {rest:?}")),
            },
            "fold" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Fold(n - 1),
                _ => return Err(format!("fold needs a number from 1, got {rest:?}")),
            },
            "library" if !rest.is_empty() => Command::Library(rest.to_string()),
            "library" => return Err("library needs a row's name".into()),
            "library-layout" => match crate::shelf::Layout::named(rest) {
                Some(layout) => Command::LibraryLayout(layout),
                None => {
                    return Err(format!(
                        "library-layout needs list, grid or shapes, got {rest:?}"
                    ));
                }
            },
            "select" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Select(n - 1),
                _ => return Err(format!("select needs a number from 1, got {rest:?}")),
            },
            "drag" => match rest.split_whitespace().collect::<Vec<_>>()[..] {
                [from, to, side] => {
                    let half = match side {
                        "above" => Half::Upper,
                        "below" => Half::Lower,
                        _ => return Err(format!("drag ends with above or below, got {side:?}")),
                    };
                    match (from.parse::<usize>(), to.parse::<usize>()) {
                        (Ok(from), Ok(to)) if from > 0 && to > 0 => {
                            Command::Drag(from - 1, to - 1, half)
                        }
                        _ => return Err(format!("drag needs two numbers from 1, got {rest:?}")),
                    }
                }
                _ => return Err(format!("drag needs N M above|below, got {rest:?}")),
            },
            "drop" => Command::Drop,
            "mode" if ["plan", "live", "start", "end"].contains(&rest) => {
                Command::Mode(rest.to_string())
            }
            "mode" => return Err(format!("mode needs plan, live, start or end, got {rest:?}")),
            "template" if !rest.is_empty() => Command::Template(rest.to_string()),
            "from-template" if !rest.is_empty() => Command::FromTemplate(rest.to_string()),
            "template" | "from-template" => return Err(format!("{word} needs a plan's name")),
            "slide" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Slide(n - 1),
                _ => return Err(format!("slide needs a number from 1, got {rest:?}")),
            },
            "film" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Film(n - 1),
                _ => return Err(format!("film needs a number from 1, got {rest:?}")),
            },
            "output" if !rest.is_empty() => Command::Output(rest.to_string()),
            "output" => return Err("output needs a screen name or automatic".into()),
            "length" => match rest
                .split_once(' ')
                .map(|(n, t)| (n.parse::<usize>(), t.trim()))
            {
                Some((Ok(n), text)) if n > 0 && !text.is_empty() => {
                    Command::Length(n - 1, text.into())
                }
                _ => {
                    return Err(format!(
                        "length needs an entry from 1 and a length, got {rest:?}"
                    ));
                }
            },
            "go-on" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::GoOn(n - 1),
                _ => return Err(format!("go-on needs a number from 1, got {rest:?}")),
            },
            "fade" => {
                let words: Vec<&str> = rest.split_whitespace().collect();
                let parsed = match words[..] {
                    [n, fade_in, fade_out] => n
                        .parse::<usize>()
                        .ok()
                        .filter(|n| *n > 0)
                        .zip(fade_in.parse::<u64>().ok())
                        .zip(fade_out.parse::<u64>().ok()),
                    _ => None,
                };
                match parsed {
                    Some(((n, fade_in), fade_out)) => Command::Fade(n - 1, fade_in, fade_out),
                    None => {
                        return Err(format!(
                            "fade needs an entry from 1, then seconds in and out, got {rest:?}"
                        ));
                    }
                }
            }
            "starts" => match lantern_engine::TimeOfDay::parse(rest) {
                Some(time) => Command::Starts(time),
                None => return Err(format!("starts needs a time of day, got {rest:?}")),
            },
            "loop" => {
                let usage = || format!("loop needs N M, or N M until 10:30, got {rest:?}");
                let words: Vec<&str> = rest.split_whitespace().collect();
                let entry = |word: &str| word.parse::<usize>().ok().filter(|n| *n > 0);
                let until = match words[..] {
                    [_, _] => None,
                    [_, _, "until", time] => {
                        Some(lantern_engine::TimeOfDay::parse(time).ok_or_else(usage)?)
                    }
                    _ => return Err(usage()),
                };
                match (entry(words[0]), entry(words[1])) {
                    (Some(from), Some(to)) => Command::Loop(from - 1, to - 1, until),
                    _ => return Err(usage()),
                }
            }
            "plan-look" if !rest.is_empty() => Command::PlanLook(rest.to_string()),
            "plan-look" => return Err("plan-look needs a colour".into()),
            "click" => match rest.parse::<usize>() {
                Ok(n) if n > 0 => Command::Click(n - 1),
                _ => return Err(format!("click needs a number from 1, got {rest:?}")),
            },
            "press" if keys::op_named(rest).is_some() => Command::Press(rest.to_lowercase()),
            "press" => return Err(format!("press needs a key name, got {rest:?}")),
            "next" => Command::Next,
            "prev" => Command::Prev,
            "black" => Command::Black,
            "clear" => Command::Clear,
            "shot" if !rest.is_empty() && !rest.contains(['/', ' ']) => {
                Command::Shot(rest.to_string())
            }
            "shot" => return Err(format!("shot needs a plain name, got {rest:?}")),
            "quit" => Command::Quit,
            "pointer" | "pointer-at-row" | "pointer-at" | "pointer-press" | "pointer-release"
            | "click-at" | "drag-at" => Command::Pointer(Pointer::parse(word, rest)?),
            other => return Err(format!("unknown command {other:?}")),
        };
        let takes_argument = matches!(
            command,
            Command::Wait(_)
                | Command::WaitReady(_)
                | Command::Text(_)
                | Command::Shot(_)
                | Command::Show(_)
                | Command::Preview(_)
                | Command::Add(_)
                | Command::Section(..)
                | Command::Slot(_)
                | Command::Path(_)
                | Command::CheckKey
                | Command::PickOnYoutube
                | Command::Picked(_)
                | Command::Font(_)
                | Command::Seek(_)
                | Command::Frame(_)
                | Command::Search(_)
                | Command::ShowHit(_)
                | Command::Type(_)
                | Command::Select(_)
                | Command::Click(_)
                | Command::Drag(..)
                | Command::Length(..)
                | Command::GoOn(_)
                | Command::Fade(..)
                | Command::PlanLook(_)
                | Command::Output(_)
                | Command::Slide(_)
                | Command::Film(_)
                | Command::Template(_)
                | Command::FromTemplate(_)
                | Command::Step(_)
                | Command::Fold(_)
                | Command::Library(_)
                | Command::LibraryLayout(_)
                | Command::Press(_)
                | Command::Open(_)
                | Command::Rename(_)
                | Command::Mode(_)
                | Command::Loop(..)
                | Command::Starts(_)
                | Command::Pointer(_)
        );
        if !takes_argument && !rest.is_empty() {
            return Err(format!("{word} takes no argument"));
        }
        Ok(command)
    }
}

/// A script command for the dashboard, which only the app's event loop
/// can carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ui {
    Type(String),
    Select(usize),
    Click(usize),
    Drag(usize, usize, Half),
    Drop,
    /// Changes mode: `plan`, `live`, or `start` and `end` for a run.
    Mode(String),
    Length(usize, String),
    GoOn(usize),
    PlanLook(String),
    Output(String),
    Slide(usize),
    Film(usize),
    Template(String),
    FromTemplate(String),
    Step(usize),
    Fold(usize),
    Library(String),
    LibraryLayout(crate::shelf::Layout),
    Path(String),
    CheckKey,
    PickOnYoutube,
    Picked(String),
    Font(String),
    Press(Op),
    Open(String),
    Rename(String),
    /// The keys held, as if pressed: for a click with Ctrl or Shift.
    Mods(iced::keyboard::Modifiers),
}

/// A parsed script and how to ask for screenshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Script {
    commands: Vec<Command>,
    /// Run with the shot name as its only argument. It returns once both
    /// windows are captured. Without it, `shot` only logs.
    shooter: Option<PathBuf>,
}

impl Script {
    /// Parses the whole script up front, so a typo fails before any window opens.
    /// Blank lines and lines starting with `#` are skipped. Relative paths
    /// are resolved against `dir`, the script's folder.
    pub fn parse(source: &str, dir: &Path, shooter: Option<PathBuf>) -> Result<Script, ParseError> {
        let commands = source
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'))
            .map(|(line, text)| {
                Command::parse(text, dir).map_err(|message| ParseError { line, message })
            })
            .collect::<Result<_, _>>()?;
        Ok(Script { commands, shooter })
    }

    /// Whether it moves the pointer, which needs sway: see [`Pointer`].
    pub fn points(&self) -> bool {
        self.commands
            .iter()
            .any(|command| matches!(command, Command::Pointer(_)))
    }

    /// Runs every command in order, blocking, until the end or `quit`.
    /// Hands dashboard commands to `ui`, and asks `place` where on the
    /// dashboard a plan entry's spot is. Calls `acted` after each command.
    /// A pointer that can't be moved stops it there.
    pub fn run(
        &self,
        engine: &Engine,
        library: &Library,
        downloads: &Arc<Downloads>,
        mut ui: impl FnMut(Ui),
        mut place: impl FnMut(Aim) -> Option<iced::Point>,
        mut acted: impl FnMut(),
    ) {
        let mut hits = Vec::new();
        let mut hand = Hand::default();
        for command in &self.commands {
            log(format!("script: {command:?}"));
            match command {
                Command::Quit => return,
                Command::Pointer(pointer) => {
                    if let Err(err) = hand.point(*pointer, &mut place, &mut ui) {
                        log(format!("script: the pointer: {err}; stopping"));
                        return;
                    }
                }
                Command::Search(text) => hits = search(engine, library, text),
                Command::ShowHit(n) => match hits.get(*n) {
                    Some(hit) => engine.show_now(hit.resource.clone()),
                    None => log(format!("script: show-hit: only {} hits", hits.len())),
                },
                Command::Type(text) => {
                    if !library.settle(Duration::from_secs(60)) {
                        log("script: type: indexing still running after 60s");
                    }
                    ui(Ui::Type(text.clone()));
                }
                Command::Select(n) => ui(Ui::Select(*n)),
                Command::Click(n) => ui(Ui::Click(*n)),
                Command::Drag(from, to, half) => ui(Ui::Drag(*from, *to, *half)),
                Command::Drop => ui(Ui::Drop),
                Command::Mode(to) => ui(Ui::Mode(to.clone())),
                Command::Slide(n) => ui(Ui::Slide(*n)),
                Command::Film(n) => ui(Ui::Film(*n)),
                Command::Template(name) => ui(Ui::Template(name.clone())),
                Command::FromTemplate(name) => ui(Ui::FromTemplate(name.clone())),
                Command::Output(name) => ui(Ui::Output(name.clone())),
                Command::Length(n, text) => ui(Ui::Length(*n, text.clone())),
                Command::GoOn(n) => ui(Ui::GoOn(*n)),
                Command::PlanLook(colour) => ui(Ui::PlanLook(colour.clone())),
                Command::Step(n) => ui(Ui::Step(*n)),
                Command::Fold(n) => ui(Ui::Fold(*n)),
                Command::Library(name) => ui(Ui::Library(name.clone())),
                Command::LibraryLayout(layout) => ui(Ui::LibraryLayout(*layout)),
                Command::Path(text) => ui(Ui::Path(text.clone())),
                Command::CheckKey => ui(Ui::CheckKey),
                Command::PickOnYoutube => ui(Ui::PickOnYoutube),
                Command::Picked(id) => ui(Ui::Picked(id.clone())),
                Command::Font(text) => ui(Ui::Font(text.clone())),
                Command::Press(key) => ui(Ui::Press(
                    keys::op_named(key).expect("checked by Command::parse"),
                )),
                Command::Open(name) => ui(Ui::Open(name.clone())),
                Command::Rename(name) => ui(Ui::Rename(name.clone())),
                _ => self.perform(command, engine, library, downloads),
            }
            acted();
        }
    }

    fn perform(
        &self,
        command: &Command,
        engine: &Engine,
        library: &Library,
        downloads: &Arc<Downloads>,
    ) {
        match command {
            Command::Wait(duration) => std::thread::sleep(*duration),
            Command::WaitReady(timeout) => {
                if !downloads.settle(*timeout) {
                    log(format!("script: wait-ready timed out after {timeout:?}"));
                }
            }
            Command::Hymn => {
                let step = engine.add_resource(None, &hymn());
                if let Err(err) = engine.go_to(step) {
                    log(format!("script: hymn: {err}"));
                }
            }
            Command::Text(body) => engine.show_now(Arc::new(inline("text", "Text", body))),
            Command::Show(target) => {
                engine.show_now(Arc::new(target.resource(engine, library, downloads)));
            }
            Command::Preview(target) => {
                engine.preview(Arc::new(target.resource(engine, library, downloads)));
            }
            Command::Add(target) => {
                engine.add_resource(None, &target.resource(engine, library, downloads));
            }
            Command::Slot(search) => {
                let action = lantern_engine::Action::Live {
                    hint: Some(lantern_engine::Query::new(search.clone())),
                    shown: Vec::new(),
                };
                engine.add(None, "Live search", action);
            }
            Command::Section(budget, title) => {
                let action = lantern_engine::Action::Section {
                    budget: *budget,
                    depth: 0,
                    last: None,
                    done_by: None,
                    color: None,
                };
                engine.add(None, title.clone(), action);
            }
            Command::Starts(time) => {
                engine.edit_plan(|plan| plan.starts = Some(*time));
            }
            Command::Loop(from, to, until) => {
                let looped = engine.edit_plan(|plan| {
                    let (a, b) = (plan.steps().get(*from)?.id, plan.steps().get(*to)?.id);
                    let looped = plan.loop_over(a, b).ok()?;
                    if let Some(time) = until {
                        let mut flow = plan.flow_of(looped)?;
                        flow.repeat = lantern_engine::Repeat::Until(*time);
                        plan.set_flow(looped, flow).ok()?;
                    }
                    Some(looped)
                });
                if looped.is_none() {
                    log(format!(
                        "script: no loop from entry {} to {}",
                        from + 1,
                        to + 1
                    ));
                }
            }
            Command::Fade(n, fade_in, fade_out) => {
                let fade = lantern_engine::Fade {
                    fade_in: lantern_engine::FadeTime::from_secs(*fade_in),
                    fade_out: lantern_engine::FadeTime::from_secs(*fade_out),
                };
                let faded = engine.edit_plan(|plan| {
                    let id = plan.steps().get(*n)?.id;
                    plan.get_mut(id)?.fade = fade;
                    Some(())
                });
                if faded.is_none() {
                    log(format!("script: fade: no entry {}", n + 1));
                }
            }
            Command::Take => log(format!("script: take -> {}", engine.take())),
            Command::Play => log(format!("script: play -> {}", engine.play())),
            Command::Pause => log(format!("script: pause -> {}", engine.pause())),
            Command::Seek(position) => log(format!("script: seek -> {}", engine.seek(*position))),
            Command::Frame(frame) => {
                log(format!("script: frame -> {}", engine.goto_frame(*frame)));
            }
            Command::Next => log(format!("script: next -> {:?}", engine.next())),
            Command::Prev => log(format!("script: prev -> {:?}", engine.prev())),
            Command::Black => {
                log(format!("script: blackout {}", engine.toggle_blackout()));
            }
            Command::Clear => engine.clear(),
            Command::Shot(name) => self.shoot(name),
            Command::Quit
            | Command::Search(_)
            | Command::ShowHit(_)
            | Command::Type(_)
            | Command::Select(_)
            | Command::Click(_)
            | Command::Drag(..)
            | Command::Drop
            | Command::Mode(_)
            | Command::Length(..)
            | Command::GoOn(_)
            | Command::PlanLook(_)
            | Command::Output(_)
            | Command::Slide(_)
            | Command::Film(_)
            | Command::Template(_)
            | Command::FromTemplate(_)
            | Command::Step(_)
            | Command::Fold(_)
            | Command::Library(_)
            | Command::LibraryLayout(_)
            | Command::Path(_)
            | Command::CheckKey
            | Command::PickOnYoutube
            | Command::Picked(_)
            | Command::Font(_)
            | Command::Press(_)
            | Command::Open(_)
            | Command::Rename(_)
            | Command::Pointer(_) => {}
        }
    }

    fn shoot(&self, name: &str) {
        let Some(shooter) = &self.shooter else {
            log(format!(
                "script: shot {name} skipped, LANTERN_SHOT is unset"
            ));
            return;
        };
        match crate::programs::quiet(std::process::Command::new(shooter))
            .arg(name)
            .status()
        {
            Ok(status) if status.success() => log(format!("script: shot {name} saved")),
            Ok(status) => log(format!("script: shot {name} failed: {status}")),
            Err(err) => log(format!("script: shot {name}: {}: {err}", shooter.display())),
        }
    }
}

/// How far one step of a glide goes, in the dashboard's pixels, and how
/// long it takes: about what a hand moving a mouse does.
const STRIDE: f64 = 16.0;
const STEP: Duration = Duration::from_millis(12);
/// How long a button is held for a click, and a key round it.
const CLICK: Duration = Duration::from_millis(50);
const HOLD: Duration = Duration::from_millis(100);

/// The mouse `scripts/e2e.sh` plugs into its sway, told what to do
/// through its FIFO; where the script left it, in sway's layout, so the
/// next move glides from there; and the key held while it is pressed.
#[derive(Default)]
struct Hand {
    mouse: Option<std::fs::File>,
    at: Option<(f64, f64)>,
    held: Option<Held>,
}

impl Hand {
    fn point(
        &mut self,
        pointer: Pointer,
        place: &mut impl FnMut(Aim) -> Option<iced::Point>,
        ui: &mut impl FnMut(Ui),
    ) -> Result<(), String> {
        match pointer {
            Pointer::To(at) => self.glide(at.x.into(), at.y.into()),
            Pointer::Toward(aim) => {
                let at = place(aim).ok_or_else(|| format!("no {aim:?} in sight"))?;
                self.glide(at.x.into(), at.y.into())
            }
            Pointer::Press(button, held) => self.press(button, held, ui),
            Pointer::Release(button) => self.release(button, ui),
            Pointer::Click(at, button, held) => {
                self.glide(at.x.into(), at.y.into())?;
                self.press(button, held, ui)?;
                std::thread::sleep(CLICK);
                self.release(button, ui)
            }
            Pointer::Drag(from, to) => {
                self.glide(from.x.into(), from.y.into())?;
                self.tell("press left")?;
                std::thread::sleep(CLICK);
                self.glide(to.x.into(), to.y.into())?;
                std::thread::sleep(CLICK);
                self.tell("release left")
            }
        }
    }

    /// `button` pressed, with `held` down first, as the app sees it.
    fn press(
        &mut self,
        button: Button,
        held: Option<Held>,
        ui: &mut impl FnMut(Ui),
    ) -> Result<(), String> {
        if let Some(held) = held {
            ui(Ui::Mods(match held {
                Held::Ctrl => iced::keyboard::Modifiers::CTRL,
                Held::Shift => iced::keyboard::Modifiers::SHIFT,
            }));
            std::thread::sleep(HOLD);
            self.held = Some(held);
        }
        self.tell(&format!("press {}", button.name()))
    }

    /// `button` let go, then the key held with its press.
    fn release(&mut self, button: Button, ui: &mut impl FnMut(Ui)) -> Result<(), String> {
        self.tell(&format!("release {}", button.name()))?;
        if self.held.take().is_some() {
            std::thread::sleep(HOLD);
            ui(Ui::Mods(iced::keyboard::Modifiers::empty()));
        }
        Ok(())
    }

    /// To `x`, `y` on the dashboard a stride at a time; the first move of
    /// all, straight there.
    fn glide(&mut self, x: f64, y: f64) -> Result<(), String> {
        let layout = Layout::now()?;
        let to = (layout.left + x, layout.top + y);
        let from = self.at.unwrap_or(to);
        let strides = ((to.0 - from.0).hypot(to.1 - from.1) / STRIDE)
            .ceil()
            .max(1.0) as u32;
        for stride in 1..=strides {
            let part = f64::from(stride) / f64::from(strides);
            let (x, y) = (
                from.0 + (to.0 - from.0) * part,
                from.1 + (to.1 - from.1) * part,
            );
            self.tell(&format!(
                "move {x:.2} {y:.2} {} {}",
                layout.wide, layout.tall
            ))?;
            std::thread::sleep(STEP);
        }
        self.at = Some(to);
        Ok(())
    }

    /// A line to the mouse, as `scripts/e2e/mouse.py` reads them.
    fn tell(&mut self, line: &str) -> Result<(), String> {
        use std::io::Write;
        if self.mouse.is_none() {
            let fifo = std::env::var_os("E2E_MOUSE").ok_or("E2E_MOUSE is not set")?;
            let mouse = std::fs::OpenOptions::new()
                .write(true)
                .open(&fifo)
                .map_err(|err| format!("the mouse at {}: {err}", Path::new(&fifo).display()))?;
            self.mouse = Some(mouse);
        }
        let mouse = self.mouse.as_mut().expect("opened above");
        writeln!(mouse, "{line}").map_err(|err| format!("the mouse: {err}"))
    }
}

impl Button {
    fn name(self) -> &'static str {
        match self {
            Button::Left => "left",
            Button::Right => "right",
        }
    }
}

/// Sway's layout as it is: how big it is, and where in it the dashboard's
/// window starts, wherever sway has put it.
struct Layout {
    wide: f64,
    tall: f64,
    left: f64,
    top: f64,
}

impl Layout {
    fn now() -> Result<Layout, String> {
        fn find(node: &serde_json::Value) -> Option<(f64, f64)> {
            let ours = node["app_id"] == crate::DASHBOARD_APP_ID
                && node["pid"].as_u64() == Some(u64::from(std::process::id()));
            if ours {
                return Some((node["rect"]["x"].as_f64()?, node["rect"]["y"].as_f64()?));
            }
            ["nodes", "floating_nodes"]
                .iter()
                .filter_map(|nodes| node[nodes].as_array())
                .flatten()
                .find_map(find)
        }
        let tree = swaymsg(&["-t", "get_tree"])?;
        let (left, top) = find(&tree).ok_or("sway has no dashboard window of this app")?;
        let size = |side: &str| {
            tree["rect"][side]
                .as_f64()
                .ok_or("sway's layout has no size")
        };
        Ok(Layout {
            wide: size("width")?,
            tall: size("height")?,
            left,
            top,
        })
    }
}

/// Why a script can't move the pointer here, if it can't: it needs the
/// sway `scripts/e2e.sh` runs it in, and the mouse plugged in there.
pub fn pointer_ready() -> Result<(), String> {
    let mouse = std::env::var_os("E2E_MOUSE").filter(|fifo| Path::new(fifo).exists());
    if std::env::var_os("SWAYSOCK").is_none() || mouse.is_none() {
        return Err(
            "pointer commands need the sway scripts/e2e.sh runs the app in, \
             with its mouse (SWAYSOCK and E2E_MOUSE are set there): \
             run the script with scripts/e2e.sh, under E2E_COMPOSITOR=sway"
                .into(),
        );
    }
    Ok(())
}

/// Asks sway for something, and reads its answer.
fn swaymsg(args: &[&str]) -> Result<serde_json::Value, String> {
    let out = std::process::Command::new("swaymsg")
        .arg("-r")
        .args(args)
        .output()
        .map_err(|err| format!("swaymsg: {err}"))?;
    if !out.status.success() {
        return Err(format!(
            "swaymsg {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout).trim()
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|err| format!("swaymsg {}: {err}", args.join(" ")))
}

/// The library's own resource for the file at `path`, once its folders
/// are indexed. A folder named from a config given by a relative path is
/// indexed by relative paths: the two are the same file where their
/// canonical paths are.
fn library_file(engine: &Engine, library: &Library, path: &Path) -> Option<Resource> {
    if !library.settle(Duration::from_secs(60)) {
        log("script: indexing still running after 60s");
    }
    let wanted = std::fs::canonicalize(path).ok()?;
    let all = Query::new("").with_limit(usize::MAX);
    engine
        .search_index(&all)
        .into_iter()
        .find(|hit| match &hit.resource.location {
            Location::File(at) => std::fs::canonicalize(at).is_ok_and(|at| at == wanted),
            _ => false,
        })
        .map(|hit| (*hit.resource).clone())
}

fn search(engine: &Engine, library: &Library, text: &str) -> Vec<Hit> {
    if !library.settle(Duration::from_secs(60)) {
        log("script: search: indexing still running after 60s");
    }
    let mut hits = Vec::new();
    for event in engine.search(Query::new(text)) {
        match event {
            SearchEvent::Hits { hits: batch, .. } => hits.extend(batch),
            SearchEvent::Failed { source, error } => {
                log(format!("script: search: {source} failed: {error}"));
            }
        }
    }
    let titles: Vec<&str> = hits.iter().map(|h| h.resource.title.as_str()).collect();
    log(format!(
        "script: search {text:?} -> {} hits: {}",
        hits.len(),
        titles.join(", ")
    ));
    hits
}

const HYMN: &str = "\
Amazing grace! How sweet the sound
That saved a wretch like me!
I once was lost, but now am found;
Was blind, but now I see.

'Twas grace that taught my heart to fear,
And grace my fears relieved;
How precious did that grace appear
The hour I first believed.

Through many dangers, toils and snares,
I have already come;
'Tis grace hath brought me safe thus far,
And grace will lead me home.";

fn hymn() -> Resource {
    inline("amazing-grace", "Amazing Grace", HYMN)
}

/// A local file as a resource, its kind guessed from the extension.
fn file(path: &Path) -> Resource {
    let source = SourceId::new("script").expect("a valid source id");
    let kind = path
        .extension()
        .and_then(|ext| Kind::from_extension(&ext.to_string_lossy()))
        .unwrap_or_else(|| Kind::Other("unknown".into()));
    let title = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    Resource::new(
        ResourceId::new(&source, &path.display().to_string()),
        title,
        kind,
        Location::File(path.to_path_buf()),
    )
}

fn inline(key: &str, title: &str, body: &str) -> Resource {
    let source = SourceId::new("script").expect("a valid source id");
    Resource::new(
        ResourceId::new(&source, key),
        title,
        Kind::Text,
        Location::Inline(body.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn run(source: &str) -> Arc<Engine> {
        run_with(Config::default(), source)
    }

    fn run_with(config: Config, source: &str) -> Arc<Engine> {
        let engine = Arc::new(Engine::new());
        let library = Library::start(engine.clone(), config);
        let downloads = Downloads::new("/nonexistent".into());
        Script::parse(source, Path::new("/media"), None)
            .unwrap()
            .run(&engine, &library, &downloads, |_| {}, |_| None, || {});
        engine
    }

    fn on_stage(engine: &Engine) -> Option<String> {
        let state = engine.stage();
        if state.blackout {
            return None;
        }
        state.program.and_then(|scene| scene.text())
    }

    #[test]
    fn hymn_steps_through_its_stanzas() {
        let engine = run("hymn\nnext\nnext");
        assert!(
            on_stage(&engine)
                .unwrap()
                .starts_with("Through many dangers")
        );
        assert_eq!(engine.plan().steps().len(), 1, "the hymn is a plan step");
    }

    #[test]
    fn black_twice_shows_the_stanza_again_and_clear_empties() {
        let engine = run("hymn\nblack");
        assert_eq!(on_stage(&engine), None);
        let engine = run("hymn\nblack\nblack");
        assert!(on_stage(&engine).unwrap().starts_with("Amazing grace"));
        let engine = run("hymn\nclear");
        assert!(engine.stage().program.is_none());
    }

    #[test]
    fn text_shows_inline_text_with_line_breaks() {
        let engine = run(r#"text "Welcome\nto church""#);
        assert_eq!(on_stage(&engine).unwrap(), "Welcome\nto church");
    }

    #[test]
    fn quit_stops_the_script() {
        let engine = run("hymn\nquit\nnext");
        assert!(on_stage(&engine).unwrap().starts_with("Amazing grace"));
    }

    #[test]
    fn fade_sets_how_an_entry_fades_and_the_next_waits_for_it() {
        let engine = run("hymn\nfade 1 2 3\nnext\nnext\nnext");
        let fade = engine.plan().steps()[0].fade;
        assert_eq!(fade.fade_in, lantern_engine::FadeTime::from_secs(2));
        assert_eq!(fade.fade_out, lantern_engine::FadeTime::from_secs(3));
        assert!(
            engine.stage().fading_out.is_some(),
            "past its last stanza, it fades out"
        );
        assert!(Script::parse("fade 1 2", Path::new(""), None).is_err());
        assert!(Script::parse("fade 0 2 3", Path::new(""), None).is_err());
    }

    #[test]
    fn starts_gives_the_plan_its_time_of_day() {
        let engine = run("hymn\nstarts 19:30");
        assert_eq!(
            engine.plan().starts.map(|time| time.to_string()),
            Some("19:30".to_string())
        );
        assert!(Script::parse("starts 25:00", Path::new(""), None).is_err());
        assert!(Script::parse("starts", Path::new(""), None).is_err());
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let script =
            Script::parse("# setup\n\n  wait 5\nshot first\n", Path::new(""), None).unwrap();
        assert_eq!(
            script.commands,
            [
                Command::Wait(Duration::from_millis(5)),
                Command::Shot("first".into())
            ]
        );
    }

    #[test]
    fn dashboard_commands_go_to_the_ui() {
        let engine = Arc::new(Engine::new());
        let library = Library::start(engine.clone(), Config::default());
        let downloads = Downloads::new("/nonexistent".into());
        let mut sent = Vec::new();
        let script = "type pa\nselect 2\nclick 3\ndrag 1 3 below\ndrop\nfold 2\nlibrary Black screen\nlibrary-layout Shapes\nfilm 3\npress PageDown\npress ctrl+s\nrename Sunday 3\nopen Sunday 3";
        Script::parse(script, Path::new(""), None).unwrap().run(
            &engine,
            &library,
            &downloads,
            |ui| sent.push(ui),
            |_| None,
            || {},
        );
        assert_eq!(
            sent,
            [
                Ui::Type("pa".into()),
                Ui::Select(1),
                Ui::Click(2),
                Ui::Drag(0, 2, Half::Lower),
                Ui::Drop,
                Ui::Fold(1),
                Ui::Library("Black screen".into()),
                Ui::LibraryLayout(crate::shelf::Layout::Shapes),
                Ui::Film(2),
                Ui::Press(Op::Next),
                Ui::Press(Op::Save),
                Ui::Rename("Sunday 3".into()),
                Ui::Open("Sunday 3".into()),
            ]
        );
    }

    #[test]
    fn media_commands_drive_the_stage() {
        let engine = run("show pictures/a.png");
        let program = engine.stage().program.unwrap();
        assert_eq!(program.resource.kind, Kind::Image);
        assert_eq!(
            program.resource.location,
            Location::File("/media/pictures/a.png".into())
        );

        let engine = run("preview clip.mp4\ntake\nplay\nseek 1.5");
        let timed = *engine.stage().program.unwrap().timed().unwrap();
        assert_eq!(timed.playback, lantern_engine::Playback::Playing);
        assert_eq!(timed.position, Duration::from_millis(1500));

        let engine = run("add a.png\nnext");
        assert_eq!(engine.stage().program.unwrap().resource.title, "a.png");
    }

    #[test]
    fn a_file_is_added_as_the_app_has_it() {
        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/e2e/fixtures");
        let added = |config: Config| {
            let engine = run_with(config, &format!("add {fixtures}/image.png\nnext"));
            let resource = engine.stage().program.unwrap().resource;
            (resource.title.clone(), resource.subtitle.clone())
        };
        let folder = format!("[[folders]]\npath = '{fixtures}'\nlabel = \"Fixtures\"");
        let folder = Config::parse(&folder, Path::new("/"), None).unwrap();
        assert_eq!(
            added(folder),
            ("image".to_string(), Some("Fixtures".to_string())),
            "in a library folder, the library's own"
        );
        let relative = "[[folders]]\npath = 'fixtures'\nlabel = \"Fixtures\"";
        let relative = Config::parse(relative, Path::new("../../scripts/e2e"), None).unwrap();
        assert_eq!(
            added(relative),
            ("image".to_string(), Some("Fixtures".to_string())),
            "a folder named from a relative config is the same folder"
        );
        assert_eq!(
            added(Config::default()),
            ("image".to_string(), Some(fixtures.to_string())),
            "elsewhere, as its pasted path would be"
        );
    }

    #[test]
    fn links_resolve_like_a_pasted_link() {
        let engine = run("add https://youtu.be/jNQXAC9IVRw\nnext");
        let resource = engine.stage().program.unwrap().resource;
        assert_eq!(resource.kind, Kind::Video);
        assert_eq!(resource.meta["youtube_id"], "jNQXAC9IVRw");
        assert_eq!(
            resource.location,
            Location::Url("https://www.youtube.com/watch?v=jNQXAC9IVRw".into())
        );
    }

    #[test]
    fn search_then_show_hit_puts_a_bible_passage_on_stage() {
        let bible = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../sources/bible/fixtures/sample.tsv"
        );
        // A literal string: a Windows path's `\` is no escape.
        let config = format!("[[bibles]]\nlang = \"es\"\npath = '{bible}'");
        let config = Config::parse(&config, Path::new("/"), None).unwrap();
        let engine = run_with(config, "search juan 3:16-17\nshow-hit 1");
        let program = engine.stage().program.unwrap();
        assert_eq!(program.resource.title, "Juan 3:16-17");
        assert!(program.text().unwrap().starts_with("Porque de tal manera"));
    }

    #[test]
    fn frame_counts_from_one() {
        let engine = run("hymn\nframe 3");
        assert!(
            on_stage(&engine)
                .unwrap()
                .starts_with("Through many dangers")
        );
    }

    #[test]
    fn pointer_commands_read_places_buttons_and_keys() {
        let read = |source| {
            Script::parse(source, Path::new(""), None)
                .map(|script| script.commands)
                .map_err(|err| err.to_string())
        };
        let at = |x, y| At { x, y };
        let pointer = |pointer| Ok(vec![Command::Pointer(pointer)]);
        assert_eq!(read("pointer 640 360"), pointer(Pointer::To(at(640, 360))));
        let toward = |aim| pointer(Pointer::Toward(aim));
        assert_eq!(read("pointer-at-row 3"), toward(Aim::Row(2, Spot::Middle)));
        assert_eq!(
            read("pointer-at-row 3 foot"),
            toward(Aim::Row(2, Spot::Foot))
        );
        assert_eq!(
            read("pointer-at-row 4 chevron"),
            toward(Aim::Row(3, Spot::Chevron))
        );
        assert_eq!(read("pointer-at title"), toward(Aim::Title));
        assert_eq!(read("pointer-at ring-note"), toward(Aim::RingNote));
        assert_eq!(read("pointer-at library 5"), toward(Aim::Library(4)));
        assert!(read("pointer-at library 0").is_err(), "counted from 1");
        assert!(read("pointer-at nowhere").is_err());
        assert_eq!(
            read("pointer-press"),
            pointer(Pointer::Press(Button::Left, None))
        );
        assert_eq!(
            read("pointer-press left ctrl"),
            pointer(Pointer::Press(Button::Left, Some(Held::Ctrl)))
        );
        assert_eq!(
            read("pointer-release right"),
            pointer(Pointer::Release(Button::Right))
        );
        assert_eq!(
            read("click-at 10 20 right ctrl"),
            pointer(Pointer::Click(at(10, 20), Button::Right, Some(Held::Ctrl)))
        );
        assert_eq!(
            read("click-at 10 20 shift"),
            pointer(Pointer::Click(at(10, 20), Button::Left, Some(Held::Shift)))
        );
        assert_eq!(
            read("drag-at 1 2 3 4"),
            pointer(Pointer::Drag(at(1, 2), at(3, 4)))
        );
        assert_eq!(
            read("pointer 640"),
            Err(r#"line 1: pointer needs X Y, got "640""#.to_string())
        );
        assert!(read("pointer-at-row 0").is_err());
        assert!(read("pointer-at-row 2 side").is_err());
        assert!(read("click-at 10 20 middle").is_err());
        assert!(read("click-at 10 20 ctrl shift").is_err());
        assert!(read("pointer-press up").is_err());
        assert!(read("pointer-release ctrl").is_err());
        assert!(read("drag-at 1 2 3").is_err());
        assert!(
            Script::parse("pointer 1 2", Path::new(""), None)
                .unwrap()
                .points()
        );
        assert!(
            !Script::parse("wait 5", Path::new(""), None)
                .unwrap()
                .points()
        );
    }

    #[test]
    fn errors_name_the_line() {
        let err = |source| {
            Script::parse(source, Path::new("/media"), None)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(err("hymn\njump"), r#"line 2: unknown command "jump""#);
        assert_eq!(
            err("wait soon"),
            r#"line 1: wait needs milliseconds, got "soon""#
        );
        assert_eq!(err("next 3"), "line 1: next takes no argument");
        assert_eq!(err("search"), "line 1: search needs text");
        assert_eq!(err("type"), "line 1: type needs text");
        assert_eq!(err("open"), "line 1: open needs a name");
        assert_eq!(
            err("library-layout tiles"),
            r#"line 1: library-layout needs list, grid or shapes, got "tiles""#
        );
        assert_eq!(
            err("select 0"),
            r#"line 1: select needs a number from 1, got "0""#
        );
        assert_eq!(
            err("press f13"),
            r#"line 1: press needs a key name, got "f13""#
        );
        assert_eq!(
            err("show-hit x"),
            r#"line 1: show-hit needs a number from 1, got "x""#
        );
        assert_eq!(err("show"), "line 1: show needs a path or link");
        assert_eq!(err("show https://"), r#"line 1: not a link: "https://""#);
        assert_eq!(
            err("wait-ready soon"),
            r#"line 1: wait-ready needs seconds, got "soon""#
        );
        assert_eq!(
            err("frame 0"),
            r#"line 1: frame needs a number from 1, got "0""#
        );
        assert_eq!(err("seek -1"), r#"line 1: seek needs seconds, got "-1""#);
        assert_eq!(
            err("text hello"),
            r#"line 1: text needs a quoted string, got "hello""#
        );
        assert_eq!(
            err("shot ../x"),
            r#"line 1: shot needs a plain name, got "../x""#
        );
    }
}
