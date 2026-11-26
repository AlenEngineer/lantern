//! Run's rail: the road the service takes, not the plan's shape. What
//! was played, what is on, then what comes, in the order it will come:
//! a loop's next round is rows of its own, a drawn order stands as it
//! was drawn. A section has a header each time the road comes to it, as
//! on the plan's rows, and its line in its colour beside its rows. A
//! folded section is one thing on it, its header, whichever of its steps
//! is on.
//!
//! As the service goes on, the road moves: the ring of what went on
//! opens and what was on fades to played, rows grow and shrink to their
//! new heights rather than jump ([`crate::settle`]), the tube fills
//! smoothly between the moments it is measured, and a loop going round
//! sends a light back up its section's line.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{Alignment, Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};
use lantern_engine::timing::Timing;
use lantern_engine::{Action, Coming, FlowOf, Plan, Step, StepId};

use crate::look::{self, BODY, BOLD, META, Tone};
use crate::motion::{Glide, blend, ease};
use crate::palette::Palette;
use crate::plan_pane::{HEAD_LINE, INDENT, RAIL, RAIL_DOT, RAIL_TOP};
use crate::settle::{Heights, settle};
use crate::track::{self, Key, Places};
use crate::{App, Message};

/// Where a thing on the road is, against what is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Played,
    On,
    Next,
    Coming,
}

/// One row of the road.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum What {
    Step(StepId),
    /// A folded section, as one row; `holds` the step of it that is on.
    Folded {
        section: StepId,
        holds: Option<StepId>,
    },
    /// The rounds of `of` from round `from` on, as one row: the road
    /// lays out the round under way and the next, no more.
    Again {
        of: FlowOf,
        from: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Thing {
    pub(crate) what: What,
    pub(crate) place: Place,
    /// Which time round it comes in, from 0.
    pub(crate) round: u16,
    /// How long from now until it starts; `None` for what was played or
    /// is on.
    pub(crate) after: Option<Duration>,
    /// Whether it goes on by itself to the thing after it.
    pub(crate) joined: bool,
    /// The innermost section round it, whose line its dot is on: for a
    /// folded section, the one round that.
    pub(crate) within: Option<StepId>,
}

/// What a row of the road stands for, wherever it is on the road: a
/// thing in the round it comes in, or a section's header there. A folded
/// section's one row is its header, whichever of its steps is on, so a
/// section that folds keeps its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Who {
    Thing(What, u16),
    Head(StepId, u16),
}

impl Thing {
    pub(crate) fn who(&self) -> Who {
        match self.what {
            What::Folded { section, .. } => Who::Head(section, self.round),
            what => Who::Thing(what, self.round),
        }
    }
}

/// The road's rows as the service goes on: how tall each is shown, the
/// rows as last laid out, and as they were before it last went on.
#[derive(Default)]
pub(crate) struct Rows {
    heights: Heights<Who>,
    laid: RefCell<Vec<Who>>,
    was: Vec<Who>,
}

impl Rows {
    /// The service went on: the rows there were, it went on from.
    pub(crate) fn went_on(&mut self) {
        self.was = self.laid.borrow().clone();
    }
}

/// The rows of `was` gone from `now`, each with the row of `now` it
/// stays before as it closes up (`now.len()` for after the last).
fn leaving(was: &[Who], now: &[Who]) -> Vec<(usize, Who)> {
    let mut before = 0;
    let mut gone = Vec::new();
    for who in was {
        match now.iter().position(|kept| kept == who) {
            Some(at) => before = at + 1,
            None => gone.push((before, *who)),
        }
    }
    gone
}

/// A section's header over a run of things next to each other that are
/// in it: the road heads a section each time it comes back to it. A
/// folded section's own row is its header, so it has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Head {
    pub(crate) section: StepId,
    pub(crate) first: usize,
    pub(crate) last: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Road {
    pub(crate) things: Vec<Thing>,
    pub(crate) heads: Vec<Head>,
}

/// A row of the road: the `h`th of its heads, or its `n`th thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shown {
    Head(usize),
    Thing(usize),
}

impl Road {
    /// Its rows, top to bottom: before each thing, the headers of the
    /// sections whose runs start there, the outermost first.
    fn shown(&self) -> Vec<Shown> {
        let mut heads = self.heads.iter().enumerate().peekable();
        let mut rows = Vec::new();
        for n in 0..self.things.len() {
            while let Some((h, _)) = heads.next_if(|(_, head)| head.first == n) {
                rows.push(Shown::Head(h));
            }
            rows.push(Shown::Thing(n));
        }
        rows
    }

    fn who(&self, row: Shown) -> Who {
        match row {
            Shown::Head(h) => {
                let head = self.heads[h];
                Who::Head(head.section, self.things[head.first].round)
            }
            Shown::Thing(n) => self.things[n].who(),
        }
    }

    /// Each section's line beside the road, in pieces, top to bottom: the
    /// section, the row a piece starts at (its header, or the first of
    /// its things after a section inside it) and the thing whose dot it
    /// ends at, the last before the next header.
    fn stretches(&self) -> Vec<(StepId, Shown, usize)> {
        let ended = |piece: Option<(StepId, Shown, Option<usize>)>| {
            piece.and_then(|(section, from, last)| Some((section, from, last?)))
        };
        let mut pieces = Vec::new();
        let mut piece = None;
        for row in self.shown() {
            if let (Shown::Thing(n), Some((section, _, last))) = (row, piece.as_mut())
                && self.things[n].within == Some(*section)
            {
                *last = Some(n);
                continue;
            }
            pieces.extend(ended(piece.take()));
            piece = match row {
                Shown::Head(h) => Some((self.heads[h].section, row, None)),
                Shown::Thing(n) => self.things[n].within.map(|section| (section, row, Some(n))),
            };
        }
        pieces.extend(ended(piece));
        pieces.retain(|(_, from, last)| *from != Shown::Thing(*last));
        pieces
    }
}

/// The road from `played` (in the order it came), the step `on`, and
/// what comes after it (from the engine's road ahead). A step in a
/// section that is `shut` is gathered with its neighbours in it into
/// the section's one row.
pub(crate) fn lay(
    plan: &Plan,
    played: &[StepId],
    on: Option<StepId>,
    ahead: &[Coming],
    shut: &dyn Fn(StepId) -> bool,
    round_now: &dyn Fn(FlowOf) -> u16,
) -> Road {
    let entries = played
        .iter()
        .map(|id| (*id, Place::Played, 0, None, None))
        .chain(on.map(|id| (id, Place::On, 0, None, None)))
        .chain(ahead.iter().enumerate().map(|(n, coming)| {
            let place = match n {
                0 => Place::Next,
                _ => Place::Coming,
            };
            (
                coming.step,
                place,
                coming.round,
                Some(coming.after),
                coming.of,
            )
        }))
        .filter_map(|(id, place, round, after, of)| {
            Some((plan.position(id)?, id, place, round, after, of))
        });
    // What goes on by itself into what follows it: an entry ahead says
    // so of the one before it.
    let joined_into: Vec<bool> = ahead.iter().map(|coming| coming.joined).collect();
    let mut things: Vec<Thing> = Vec::new();
    // The plan row each thing stands for: a step's own, a folded
    // section's heading.
    let mut rows: Vec<usize> = Vec::new();
    let first_ahead = played.len() + usize::from(on.is_some());
    for (n, (at, id, place, round, after, of)) in entries.enumerate() {
        let joined = n + 1 >= first_ahead
            && joined_into
                .get(n + 1 - first_ahead)
                .copied()
                .unwrap_or(false);
        // Past the round under way and the next, what comes round again
        // is one row.
        if let Some(of) = of
            && round >= round_now(of) + 2
        {
            if let Some(last) = things.last_mut()
                && matches!(last.what, What::Again { of: o, .. } if o == of)
            {
                last.joined = joined;
                continue;
            }
            things.push(Thing {
                what: What::Again {
                    of,
                    from: round_now(of) + 2,
                },
                place,
                round,
                after,
                joined,
                within: None,
            });
            rows.push(at);
            continue;
        }
        let folded = plan
            .within(at)
            .into_iter()
            .find(|section| shut(*section))
            .and_then(|section| Some((section, plan.position(section)?)));
        let Some((section, heading)) = folded else {
            things.push(Thing {
                what: What::Step(id),
                place,
                round,
                after,
                joined,
                within: None,
            });
            rows.push(at);
            continue;
        };
        // Gathered with the thing before it when that is the same
        // section, folded.
        if let Some(last) = things.last_mut()
            && matches!(last.what, What::Folded { section: s, .. } if s == section)
        {
            last.place = match (last.place, place) {
                (Place::On, _) | (_, Place::On) => Place::On,
                (Place::Played, Place::Played) => Place::Played,
                (Place::Played, later) => later,
                (earlier, _) => earlier,
            };
            if let What::Folded { holds, .. } = &mut last.what
                && place == Place::On
            {
                *holds = Some(id);
            }
            last.after = last.after.or(after);
            last.joined = joined;
            continue;
        }
        things.push(Thing {
            what: What::Folded {
                section,
                holds: (place == Place::On).then_some(id),
            },
            place,
            round,
            after,
            joined,
            within: None,
        });
        rows.push(heading);
    }
    // The sections each thing is shown in, the outermost first: a folded
    // one's own last.
    let mut shown_in: Vec<Vec<StepId>> = Vec::new();
    for (thing, &at) in things.iter_mut().zip(&rows) {
        let mut sections = plan.within(at);
        thing.within = sections.last().copied();
        if let What::Folded { section, .. } = thing.what {
            sections.push(section);
        }
        shown_in.push(sections);
    }
    let mut heads = Vec::new();
    for (n, sections) in shown_in.iter().enumerate() {
        for section in sections {
            // Only where a run of it starts here, and not over a folded
            // section's own row, its header.
            let its_own = matches!(things[n].what, What::Folded { section: folded, .. } if folded == *section);
            if its_own || (n > 0 && shown_in[n - 1].contains(section)) {
                continue;
            }
            let mut last = n;
            while last + 1 < things.len() && shown_in[last + 1].contains(section) {
                last += 1;
            }
            heads.push(Head {
                section: *section,
                first: n,
                last,
            });
        }
    }
    Road { things, heads }
}

/// How the road is drawn beside its rows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Solid(Color),
    Hollow(Color),
    /// What is on, its ring `open` of the way out; a glow going out of it
    /// (`glow` of the way gone) as the service goes round what it is in.
    On {
        color: Color,
        open: f32,
        glow: Option<f32>,
    },
    Stack {
        color: Color,
        solid: bool,
    },
}

/// What runs from one row's dot to the next one's.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Link {
    /// Thin: what was played, or what goes on by itself.
    Plain(Color),
    /// From what is on to what is next: `full` of it is filled.
    Tube {
        full: f32,
        color: Color,
        waits: bool,
    },
    /// It waits for the operator here.
    Waits(Color),
}

/// A section's line beside its rows, at half strength: from `from`, its
/// header or the dot of its first thing after a section inside it, down
/// to the dot of its `to`th thing.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Stretch {
    from: Key,
    to: usize,
    color: Color,
}

/// A light run `t` of the way back up a section's line as the service
/// went round it: from the dot of its `last` thing to its `head`th
/// header, where the round begins.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Pulse {
    head: usize,
    last: usize,
    color: Color,
    t: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    marks: Vec<Mark>,
    /// From each thing to the one after it.
    links: Vec<Option<Link>>,
    /// The thing each header is over.
    heads: Vec<usize>,
    stretches: Vec<Stretch>,
    pulse: Option<Pulse>,
    places: Places,
    /// What is on, whose tube fills.
    on: Option<Who>,
    /// The sounds that keep going: from the row of the step that set
    /// one to the row of the last it keeps going under, and how much of
    /// it has played while it plays.
    sounds: Vec<(usize, usize, Option<f32>)>,
    /// Nothing moves.
    still: bool,
}

impl App {
    /// How long the step that is on has been up, and how long it has
    /// left: by the stage for what plays in time, else by its length.
    fn on_for(&self, step: &Step) -> (Duration, Duration) {
        let scene = self
            .stage
            .program
            .as_ref()
            .filter(|scene| match &step.action {
                Action::Show { resource } => scene.resource.id == resource.id,
                _ => false,
            });
        if let Some(timed) = scene.and_then(|scene| scene.timed())
            && let Some(duration) = timed.duration.filter(|d| !d.is_zero())
        {
            return (timed.position, duration.saturating_sub(timed.position));
        }
        let length = self.estimate(step).length;
        let up = match (scene, self.shown_since) {
            (Some(scene), Some((Some(shown), frame, since))) if shown == scene.id => {
                let frames = crate::frame_count(scene).max(1);
                length / u32::try_from(frames).unwrap_or(1) * u32::try_from(frame).unwrap_or(0)
                    + since.elapsed()
            }
            _ => match self.shown_since {
                Some((None, _, since)) => since.elapsed(),
                _ => Duration::ZERO,
            },
        };
        (up.min(length), length.saturating_sub(up))
    }

    /// How far through the step that is on is: by the stage for what
    /// plays in time or has slides, else by its length.
    fn part_done(&self, step: &Step) -> f32 {
        let shown = self
            .stage
            .program
            .as_ref()
            .filter(|scene| match &step.action {
                Action::Show { resource } => scene.resource.id == resource.id,
                _ => false,
            });
        if let Some((_, done)) =
            shown.and_then(|scene| crate::plan_pane::progress(&scene.media, &scene.resource))
        {
            return done;
        }
        let (up, left) = self.on_for(step);
        let whole = (up + left).as_secs_f32();
        match whole > 0.0 {
            true => (up.as_secs_f32() / whole).clamp(0.0, 1.0),
            false => 0.0,
        }
    }

    /// How far through its steps `section` is, with `on` the step that
    /// is on: [`through`], with what has a length of its own timed.
    fn through_section(&self, plan: &Plan, section: StepId, on: &Step, drawn: &[StepId]) -> f32 {
        let Some(at) = plan.position(section) else {
            return 0.0;
        };
        let steps: Vec<&Step> = plan.steps()[at + 1..plan.group_end(at)]
            .iter()
            .filter(|step| !step.is_section())
            .collect();
        let Some(now) = steps.iter().position(|step| step.id == on.id) else {
            return 0.0;
        };
        let shares: Vec<(Option<Duration>, Duration)> = steps
            .iter()
            .map(|step| (self.loaded_length(step), self.estimate(step).length))
            .collect();
        // Played: before it this round, or drawn before it in a shuffle.
        let played: Vec<bool> = steps
            .iter()
            .enumerate()
            .map(|(k, step)| k < now || drawn.contains(&step.id))
            .collect();
        let round = through(&shares, &played, now, self.part_done(on));
        // A section that comes round a number of times fills over all of
        // them.
        match plan
            .section_span(section)
            .map(|span| self.engine.0.rounds(lantern_engine::FlowOf::Span(span.id)))
        {
            Some((done, Some(times))) if times > 1 => {
                ((f32::from(done) + round) / f32::from(times)).clamp(0.0, 1.0)
            }
            _ => round,
        }
    }

    /// Run's rail while the service is watched: the road it takes, each
    /// thing a row, with its dot and the tube to what is next; over each
    /// run of a section's things its header, and its line in its colour
    /// beside them.
    pub(crate) fn road_rail<'a>(
        &'a self,
        plan: &Plan,
        timing: &Timing,
        issues: &[crate::ready::Issue],
    ) -> Element<'a, Message> {
        let p = self.palette;
        let engine = &self.engine.0;
        let current = engine.current_step();
        let at = current.as_ref().and_then(|step| plan.position(step.id));
        let drawn = engine.drawn_before();
        let played: Vec<StepId> = plan
            .steps()
            .iter()
            .enumerate()
            .filter(|(index, step)| {
                !step.is_section()
                    && Some(step.id) != current.as_ref().map(|c| c.id)
                    && (at.is_some_and(|at| *index < at) || drawn.contains(&step.id))
                    && engine.reached(step.id)
            })
            .map(|(_, step)| step.id)
            .collect();
        let left = current
            .as_ref()
            .map_or(Duration::ZERO, |step| self.on_for(step).1);
        // Far enough to see past a loop that goes round until a time:
        // its later rounds are one row.
        let ahead = engine.ahead(|step| self.estimate(step).length, left, 400);
        let road = lay(
            plan,
            &played,
            current.as_ref().map(|step| step.id),
            &ahead,
            &|id| self.shut(timing, id),
            &|of| engine.round_of(of),
        );
        let colors = plan.section_colors();
        let hue = |id: StepId| crate::plan_pane::section_hue(p, &colors, id);
        let places = self.places.clone();
        let motion = &self.motion;
        let rows = &self.road_rows;
        let shown = road.shown();
        let whos: Vec<Who> = shown.iter().map(|row| road.who(*row)).collect();
        // Just gone on: what comes in grows, and what went closes up.
        let stepping = motion.stepped().is_some();
        let gone = match stepping {
            true => leaving(&rows.was, &whos),
            false => Vec::new(),
        };
        rows.heights
            .borrow_mut()
            .retain(|who, _| whos.contains(who) || gone.iter().any(|(_, left)| left == who));
        rows.laid.replace(whos.clone());
        let closing = |list: iced::widget::Column<'a, Message>, before: usize| {
            gone.iter()
                .filter(|(at, _)| *at == before)
                .fold(list, |list, (_, who)| {
                    list.push(settle(
                        *who,
                        &rows.heights,
                        false,
                        motion.still,
                        Space::new(),
                    ))
                })
        };
        // As the service goes round a section, a light runs back up its
        // line from the round under way; round anything else, the dot of
        // what is on glows.
        let round = motion.going_round();
        let on_at = road
            .things
            .iter()
            .position(|thing| thing.place == Place::On);
        let rounded = |head: &Head| {
            let (of, t) = round?;
            let goes_round = of == FlowOf::Step(head.section)
                || plan
                    .section_span(head.section)
                    .is_some_and(|span| of == FlowOf::Span(span.id));
            let under_way = on_at.is_some_and(|on| (head.first..=head.last).contains(&on));
            (goes_round && under_way).then_some(t)
        };
        let glow = round
            .filter(|_| !road.heads.iter().any(|head| rounded(head).is_some()))
            .map(|(_, t)| t);
        let opened = motion.stepped().map_or(1.0, ease);
        // What is still to come is in the colour of the section it is in,
        // a folded section in its own.
        let planned = |thing: &Thing| match thing.what {
            What::Folded { section, .. } => hue(section),
            _ => thing.within.map_or(p.primary, hue),
        };
        let mut list = column![];
        let mut marks = Vec::new();
        for (k, row) in shown.iter().enumerate() {
            list = closing(list, k);
            let content: Element<'a, Message> = match *row {
                Shown::Head(h) => {
                    let head = road.heads[h];
                    // Gone round, its name says the new round brightly.
                    let color = match rounded(&head) {
                        Some(t) => blend(Color::WHITE, hue(head.section), t),
                        None => hue(head.section),
                    };
                    let round = road.things[head.first].round;
                    self.road_head(plan, timing, head.section, round, color, Key::RoadHead(h))
                }
                Shown::Thing(n) => {
                    let thing = road.things[n];
                    marks.push(match (thing.what, thing.place) {
                        (What::Again { .. }, _) => Mark::Stack {
                            color: p.on_surface_variant,
                            solid: false,
                        },
                        (What::Folded { section, .. }, Place::Played) => Mark::Stack {
                            color: Palette::tint(hue(section), 0.45),
                            solid: true,
                        },
                        (What::Folded { .. }, Place::On) => Mark::Stack {
                            color: p.tertiary,
                            solid: true,
                        },
                        (What::Folded { .. }, Place::Next) => Mark::Stack {
                            color: p.tertiary,
                            solid: false,
                        },
                        (What::Folded { section, .. }, Place::Coming) => Mark::Stack {
                            color: hue(section),
                            solid: false,
                        },
                        // What was on fades to played, its ring closing.
                        (What::Step(id), Place::Played) => match motion.going_off(id).map(ease) {
                            Some(t) => Mark::On {
                                color: blend(p.tertiary, p.outline, t),
                                open: 1.0 - t,
                                glow: None,
                            },
                            None => Mark::Solid(p.outline),
                        },
                        (_, Place::On) => Mark::On {
                            color: p.tertiary,
                            open: opened,
                            glow,
                        },
                        (_, Place::Next) => Mark::Hollow(p.tertiary),
                        (_, Place::Coming) if thing.joined => Mark::Solid(planned(&thing)),
                        (_, Place::Coming) => Mark::Hollow(planned(&thing)),
                    });
                    let dot =
                        track::seen(Key::Road(n), &places, Space::new().width(RAIL).height(Fill));
                    let thing = self.road_thing(plan, timing, issues, thing, &hue);
                    row![dot, container(thing).width(Fill)]
                        .height(Length::Shrink)
                        .into()
                }
            };
            // The room between rows is part of each, so one closing up
            // takes its room with it.
            let whole = container(content).padding(iced::Padding::default().bottom(4.0));
            // Two rows for one thing would pull its height two ways.
            list = match whos.iter().position(|who| *who == whos[k]) == Some(k) {
                true => list.push(settle(
                    whos[k],
                    &rows.heights,
                    stepping,
                    motion.still,
                    whole,
                )),
                false => list.push(whole),
            };
        }
        let list = closing(list, shown.len());
        // From each thing to the next.
        let links = (0..road.things.len())
            .map(|n| {
                let thing = road.things[n];
                let next = road.things.get(n + 1)?;
                Some(match (thing.place, next.place) {
                    (Place::Played, _) => Link::Plain(p.outline),
                    (Place::On, _) => {
                        // A step fills as far as it is through; a folded
                        // section as far as it is through its steps.
                        let full = match (thing.what, current.as_ref()) {
                            (What::Folded { section, .. }, Some(on)) => {
                                self.through_section(plan, section, on, &drawn)
                            }
                            (_, Some(on)) => self.part_done(on),
                            (_, None) => 0.0,
                        };
                        Link::Tube {
                            full,
                            color: p.tertiary,
                            waits: !thing.joined,
                        }
                    }
                    _ if thing.joined => Link::Plain(planned(&thing)),
                    _ => Link::Waits(p.on_surface_variant),
                })
            })
            .collect();
        // Each section's line, in its colour, quiet once it is over.
        let stretches = road
            .stretches()
            .into_iter()
            .map(|(section, from, to)| Stretch {
                from: match from {
                    Shown::Head(h) => Key::RoadHead(h),
                    Shown::Thing(n) => Key::Road(n),
                },
                to,
                color: match self.over(timing, section) {
                    true => Palette::tint(hue(section), 0.45),
                    false => hue(section),
                },
            })
            .collect();
        let pulse = road.heads.iter().enumerate().find_map(|(h, head)| {
            Some(Pulse {
                head: h,
                last: head.last,
                color: hue(head.section),
                t: rounded(head)?,
            })
        });
        // A sound that keeps going, from where its step is on the road
        // (the last time, up to what is on) to where it stops.
        let row_of = |id: StepId, from: usize| {
            road.things[from..]
                .iter()
                .position(|thing| thing.what == What::Step(id))
                .map(|n| n + from)
        };
        let sounds = plan
            .steps()
            .iter()
            .filter_map(|step| {
                let until = step.sound.as_ref()?.until?;
                let first = (0..road.things.len())
                    .rev()
                    .filter(|n| on_at.is_none_or(|on| *n <= on))
                    .find(|n| road.things[*n].what == What::Step(step.id))
                    .or_else(|| row_of(step.id, 0))?;
                let last = row_of(until, first + 1)?;
                Some((first, last, self.apart_played(step.id)))
            })
            .collect();
        let drawn = Drawn {
            sounds,
            marks,
            links,
            heads: road.heads.iter().map(|head| head.first).collect(),
            stretches,
            pulse,
            places: places.clone(),
            on: on_at.map(|on| road.things[on].who()),
            still: motion.still,
        };
        stack![list, iced::widget::canvas(drawn).width(Fill).height(Fill)]
            .width(Fill)
            .into()
    }

    /// What a thing on the road shows beside its dot: a step's row, a
    /// folded section's header ([`App::road_folded`]), or the rounds of a
    /// loop past the next.
    fn road_thing<'a>(
        &'a self,
        plan: &Plan,
        timing: &Timing,
        issues: &[crate::ready::Issue],
        thing: Thing,
        hue: &dyn Fn(StepId) -> Color,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let place = match thing.place {
            Place::Played => crate::plan_pane::Place::Done,
            Place::On => crate::plan_pane::Place::Current,
            Place::Next => crate::plan_pane::Place::Next,
            Place::Coming => crate::plan_pane::Place::Later,
        };
        match thing.what {
            What::Step(id) => {
                let Some(step) = plan.get(id) else {
                    return Space::new().into();
                };
                let issue = issues.iter().find(|issue| issue.step == id);
                let noted = crate::notes::numbers_on(plan, lantern_engine::NoteOn::Step(id));
                let mut body = column![self.step_row(step, place, issue, None, &noted, false)];
                if thing.place == Place::On {
                    for (_, n, words) in
                        crate::notes::on_step(plan, id, true, &|id| self.engine.0.reached(id))
                    {
                        body = body.push(
                            container(crate::notes::said(p, n, words))
                                .padding(iced::Padding::default().top(4.0)),
                        );
                    }
                }
                match thing.round {
                    0 => body.into(),
                    round => row![
                        container(body).width(Fill),
                        text(again(round)).size(META).font(BOLD).color(p.tertiary)
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .into(),
                }
            }
            What::Folded { section, .. } => self.road_folded(plan, timing, thing, hue(section)),
            What::Again { of, from } => {
                let words = plan
                    .flow_of(of)
                    .map_or(String::new(), |flow| again_words(flow.repeat, from));
                container(text(words).size(BODY).color(p.on_surface_variant))
                    .padding([12, 10])
                    .into()
            }
        }
    }

    /// A section's header where the road comes to it, saying where it is
    /// by `key`: its line as on the plan's rows, in `color`, as far in as
    /// the section is deep. Heading the service's `round`th time round
    /// it, it says which. Twice, it folds.
    fn road_head<'a>(
        &'a self,
        plan: &Plan,
        timing: &Timing,
        section: StepId,
        round: u16,
        color: Color,
        key: Key,
    ) -> Element<'a, Message> {
        let Some(index) = plan.position(section) else {
            return Space::new().into();
        };
        let depth = plan.steps()[index].depth().unwrap_or(0);
        let line = container(self.head_line(plan, timing, index, color, false, round))
            .width(Fill)
            .padding(
                iced::Padding::default()
                    .left(10.0 + INDENT * f32::from(depth))
                    .right(10.0),
            )
            .center_y(HEAD_LINE);
        let head = row![
            Space::new().width(RAIL),
            column![Space::new().height(LEVEL - HEAD_LINE / 2.0), line],
        ];
        mouse_area(track::seen(key, &self.places, head))
            .on_double_click(Message::Fold(section))
            .interaction(mouse::Interaction::Pointer)
            .into()
    }

    /// A folded section's one row on the road, its header: its line as
    /// on the plan's rows, in `color`, level with the stacked dot; while
    /// what is on is in it, which of its steps that is, how far in, and
    /// how far in that is. Pressed, it opens.
    fn road_folded<'a>(
        &'a self,
        plan: &Plan,
        timing: &Timing,
        thing: Thing,
        color: Color,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let What::Folded { section, holds } = thing.what else {
            return Space::new().into();
        };
        let Some(at) = plan.position(section) else {
            return Space::new().into();
        };
        let line = self.head_line(plan, timing, at, color, true, thing.round);
        let mut body = column![container(line).width(Fill).center_y(HEAD_LINE)];
        if let Some(on) = holds.and_then(|id| plan.get(id)) {
            let steps: Vec<&Step> = plan.steps()[at + 1..plan.group_end(at)]
                .iter()
                .filter(|step| !step.is_section())
                .collect();
            let k = steps
                .iter()
                .position(|step| step.id == on.id)
                .map_or(1, |k| k + 1);
            let (up, _) = self.on_for(on);
            body = body.push(
                crate::widgets::single_line(format!(
                    "{} · {k} of {} · {} / {}",
                    on.label,
                    steps.len(),
                    crate::length::clock(up),
                    crate::length::clock(self.estimate(on).length)
                ))
                .size(META)
                .font(BOLD)
                .color(p.tertiary),
            );
        }
        let room = LEVEL - HEAD_LINE / 2.0;
        let depth = plan.steps()[at].depth().unwrap_or(0);
        let tone = match thing.place {
            Place::On => Tone::Live,
            _ => Tone::Plain,
        };
        mouse_area(
            button(body)
                .width(Fill)
                .padding(iced::Padding {
                    top: room,
                    bottom: room,
                    left: 10.0 + INDENT * f32::from(depth),
                    right: 10.0,
                })
                .on_press(Message::Fold(section))
                .style(look::row_filled(p, look::row_fill(p, tone))),
        )
        .interaction(mouse::Interaction::Pointer)
        .into()
    }
}

/// How far through its steps a section is, 0 to 1. Each of `steps` is
/// its own length where it has one, and how long it is planned to take.
/// One with its own length takes its part of the section's planned
/// time; the rest share what is left equally. Each step `played` counts
/// whole, and the one at `now` as far as `part` of it.
pub(crate) fn through(
    steps: &[(Option<Duration>, Duration)],
    played: &[bool],
    now: usize,
    part: f32,
) -> f32 {
    let planned: f32 = steps.iter().map(|(_, length)| length.as_secs_f32()).sum();
    let timed: f32 = steps
        .iter()
        .filter_map(|(own, _)| own.map(|own| own.as_secs_f32()))
        .sum();
    let untimed = steps.iter().filter(|(own, _)| own.is_none()).count();
    let share = |k: usize| match (planned > 0.0, steps[k].0) {
        (false, _) => 1.0 / steps.len() as f32,
        (true, Some(own)) => own.as_secs_f32() / planned.max(timed),
        (true, None) => (planned - timed).max(0.0) / untimed as f32 / planned,
    };
    let done: f32 = (0..steps.len())
        .filter(|k| *k != now && played.get(*k).copied().unwrap_or(false))
        .map(share)
        .sum();
    (done + share(now) * part.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// What the row for the later rounds of a loop says: how many more from
/// round `from` (counted from 0), until when, or for ever.
fn again_words(repeat: lantern_engine::Repeat, from: u16) -> String {
    match repeat {
        lantern_engine::Repeat::Times(times) => match times.saturating_sub(from) {
            1 => "again, once more".to_string(),
            more => format!("again ×{more}"),
        },
        lantern_engine::Repeat::Until(time) => format!("again until {time}"),
        lantern_engine::Repeat::Forever => "again, for ever".to_string(),
        lantern_engine::Repeat::Once => String::new(),
    }
}

/// "2nd", "3rd": which time round, for a step that comes again.
fn again(round: u16) -> String {
    let n = round + 1;
    let tail = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{tail}")
}

/// How far down a row of the road its dot is, level with its name; a
/// header's words are level with it too, and its line starts there.
const LEVEL: f32 = RAIL_TOP + RAIL_DOT / 2.0;
/// A dot's radius, a ring's room round the one that is on, and a
/// tube's width.
const DOT: f32 = 5.0;
const RING: f32 = 4.0;
const TUBE: f32 = 8.0;
/// How long the tube takes to fill to where it was last measured: as
/// long as it is till the next measure, so between them it fills evenly.
const GLIDE: Duration = Duration::from_millis(500);
/// How much of the tube it fills before it is drawn again, as it fills
/// evenly: less than a pixel.
const FILL_STEP: f32 = 0.004;

/// The tube of what is on, as it is shown filling.
#[derive(Debug, Default)]
struct Tube(Option<(Who, Glide)>);

impl Tube {
    /// What is on now is `on`, and its tube should be `full`. The tube
    /// of something that has just gone on fills from empty; one that is
    /// measured again goes on from where it is shown.
    fn towards(&mut self, on: Option<(Who, f32)>, now: Instant, still: bool) {
        self.0 = on.map(|(who, full)| {
            let glide = match self.0 {
                _ if still => Glide::at(full, now),
                Some((was, glide)) if was == who && glide.to == full => glide,
                Some((was, glide)) if was == who => glide.towards(full, now, GLIDE, false),
                Some(_) => Glide::new(0.0, full, now, GLIDE, true),
                // As it first shows, it is as full as it is.
                None => Glide::at(full, now),
            };
            (who, glide)
        });
    }

    /// When to draw it again: at once while it settles, else once it has
    /// filled enough to be seen to.
    fn next_look(&self, now: Instant) -> Option<Instant> {
        self.0
            .and_then(|(_, glide)| glide.when_moved(FILL_STEP, now))
    }

    fn shown(&self, on: Option<Who>, full: f32, now: Instant) -> f32 {
        match self.0 {
            Some((who, glide)) if Some(who) == on => glide.shown(now),
            _ => full,
        }
        .clamp(0.0, 1.0)
    }
}

impl Drawn {
    /// What is on, and how full its tube should be.
    fn filling(&self) -> Option<(Who, f32)> {
        let full = self.links.iter().flatten().find_map(|link| match link {
            Link::Tube { full, .. } => Some(*full),
            _ => None,
        })?;
        Some((self.on?, full))
    }
}

impl canvas::Program<Message> for Drawn {
    type State = Tube;

    fn update(
        &self,
        tube: &mut Tube,
        event: &canvas::Event,
        _: Rectangle,
        _: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let canvas::Event::Window(iced::window::Event::RedrawRequested(now)) = event else {
            return None;
        };
        tube.towards(self.filling(), *now, self.still);
        tube.next_look(*now).map(canvas::Action::request_redraw_at)
    }

    fn draw(
        &self,
        tube: &Tube,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let now = Instant::now();
        let places = self.places.borrow().clone();
        let at = |key: Key| {
            places.get(&key).map(|place| Rectangle {
                x: place.x - bounds.x,
                y: place.y - bounds.y,
                ..*place
            })
        };
        // Where a row's dot is, or a header's line starts.
        let level = |key: Key| at(key).map(|row| Point::new(row.x + RAIL / 2.0, row.y + LEVEL));
        let middle = |n: usize| level(Key::Road(n));
        // The top of the header over a thing, the outermost.
        let head_top = |n: usize| {
            let h = self.heads.iter().position(|first| *first == n)?;
            at(Key::RoadHead(h)).map(|head| head.y)
        };
        let reach = |mark: Mark| match mark {
            Mark::On { open, .. } => DOT + RING * open,
            _ => DOT,
        };
        let mut frame = Frame::new(renderer, bounds.size());
        // Each section's line at half strength, under the links.
        for stretch in &self.stretches {
            let (Some(top), Some(foot)) = (level(stretch.from), middle(stretch.to)) else {
                continue;
            };
            if foot.y > top.y {
                frame.stroke(
                    &Path::line(top, Point::new(top.x, foot.y)),
                    Stroke::default().with_width(2.0).with_color(Color {
                        a: stretch.color.a * 0.5,
                        ..stretch.color
                    }),
                );
            }
        }
        // The links, under the dots.
        for (n, link) in self.links.iter().enumerate() {
            let (Some(link), Some(from), Some(to)) = (link, middle(n), middle(n + 1)) else {
                continue;
            };
            let top = from.y + reach(self.marks[n]) + 2.0;
            let foot = to.y - reach(self.marks[n + 1]) - 2.0;
            if foot <= top {
                continue;
            }
            // Where it waits: between the rows, over a header between them
            // so it is not taken for the header's.
            let waits = match head_top(n + 1) {
                Some(head) if head > top => (top + head) / 2.0,
                _ => (top + foot) / 2.0,
            };
            match *link {
                Link::Plain(color) => frame.stroke(
                    &Path::line(Point::new(from.x, top), Point::new(from.x, foot)),
                    Stroke::default().with_width(2.0).with_color(color),
                ),
                Link::Tube {
                    full,
                    color,
                    waits: stops,
                } => {
                    let full = tube.shown(self.on, full, now);
                    let shape = Path::rounded_rectangle(
                        Point::new(from.x - TUBE / 2.0, top),
                        iced::Size::new(TUBE, foot - top),
                        (TUBE / 2.0).into(),
                    );
                    frame.stroke(
                        &shape,
                        Stroke::default()
                            .with_width(1.4)
                            .with_color(Color { a: 0.7, ..color }),
                    );
                    let filled = (foot - top) * full;
                    if filled > 1.0 {
                        frame.fill(
                            &Path::rounded_rectangle(
                                Point::new(from.x - TUBE / 2.0, top),
                                iced::Size::new(TUBE, filled),
                                (TUBE / 2.0).min(filled / 2.0).into(),
                            ),
                            Color { a: 0.85, ..color },
                        );
                    }
                    if stops && full > 0.999 {
                        pause(&mut frame, Point::new(from.x + 12.0, waits), color);
                    }
                }
                Link::Waits(color) => pause(&mut frame, Point::new(from.x, waits), color),
            }
        }
        for (n, mark) in self.marks.iter().enumerate() {
            let Some(at) = middle(n) else { continue };
            match *mark {
                Mark::Solid(color) => frame.fill(&Path::circle(at, DOT), color),
                Mark::Hollow(color) => frame.stroke(
                    &Path::circle(at, DOT - 1.0),
                    Stroke::default().with_width(2.0).with_color(color),
                ),
                Mark::On { color, open, glow } => {
                    if let Some(gone) = glow {
                        frame.stroke(
                            &Path::circle(at, DOT + 1.5 + 9.0 * gone),
                            Stroke::default().with_width(2.5).with_color(Color {
                                a: 0.6 * (1.0 - gone),
                                ..color
                            }),
                        );
                    }
                    frame.stroke(
                        &Path::circle(at, DOT + (RING - 0.75) * open),
                        Stroke::default().with_width(1.5).with_color(Color {
                            a: 0.55 * open,
                            ..color
                        }),
                    );
                    frame.fill(&Path::circle(at, DOT), color);
                }
                Mark::Stack { color, solid } => {
                    for (aside, fade) in [(-5.0, 0.35), (0.0, 0.6), (5.0, 1.0)] {
                        let dot = Point::new(at.x + aside, at.y);
                        let ink = Color {
                            a: color.a * fade,
                            ..color
                        };
                        if solid {
                            frame.fill(&Path::circle(dot, DOT - 0.5), ink);
                        }
                        frame.stroke(
                            &Path::circle(dot, DOT - 1.0),
                            Stroke::default().with_width(1.6).with_color(ink),
                        );
                    }
                }
            }
        }
        for (first, last, played) in &self.sounds {
            if let (Some(from), Some(to)) = (middle(*first), middle(*last)) {
                let from = Point::new(from.x + RING, from.y);
                track::sound_line(
                    &mut frame,
                    from,
                    to.y,
                    crate::plan_pane::SOUND_LINE,
                    *played,
                );
            }
        }
        // Gone round: a light runs back up its section's line, to where
        // the round begins.
        if let Some(Pulse {
            head,
            last,
            color,
            t,
        }) = self.pulse
            && let (Some(top), Some(foot)) = (level(Key::RoadHead(head)), middle(last))
        {
            let spot = Point::new(top.x, foot.y - (foot.y - top.y) * t);
            let fade = 1.0 - t * t;
            let light = blend(color, Color::WHITE, 0.7);
            for (size, alpha) in [(9.0, 0.18), (5.5, 0.35), (3.5, 1.0)] {
                frame.fill(
                    &Path::circle(spot, size),
                    Color {
                        a: light.a * alpha * fade,
                        ..light
                    },
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

/// The mark of waiting for the operator: two bars, their middle at `at`.
fn pause(frame: &mut Frame, at: Point, color: Color) {
    for aside in [-3.5, 1.0] {
        frame.fill_rectangle(
            Point::new(at.x + aside, at.y - 5.0),
            iced::Size::new(2.5, 10.0),
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lantern_engine::{Action, Session};

    fn names(plan: &Plan, road: &Road) -> String {
        let label = |id: StepId| plan.get(id).unwrap().label.clone();
        road.things
            .iter()
            .map(|thing| {
                let mark = match thing.place {
                    Place::Played => "-",
                    Place::On => "*",
                    Place::Next => ">",
                    Place::Coming => "",
                };
                let name = match thing.what {
                    What::Step(id) => label(id),
                    What::Folded { section, holds } => match holds {
                        Some(on) => format!("[{}:{}]", label(section), label(on)),
                        None => format!("[{}]", label(section)),
                    },
                    What::Again { from, .. } => format!("again{from}"),
                };
                let round = match thing.round {
                    0 => String::new(),
                    n => format!("'{n}"),
                };
                format!("{mark}{name}{round}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn heads(plan: &Plan, road: &Road) -> String {
        road.heads
            .iter()
            .map(|head| {
                let name = &plan.get(head.section).unwrap().label;
                format!("{name}{}-{}", head.first, head.last)
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Each piece of a section's line: from its header (h) or a thing,
    /// to the thing whose dot it ends at.
    fn stretches(plan: &Plan, road: &Road) -> String {
        road.stretches()
            .into_iter()
            .map(|(section, from, last)| {
                let name = &plan.get(section).unwrap().label;
                let from = match from {
                    Shown::Head(_) => "h".to_string(),
                    Shown::Thing(n) => n.to_string(),
                };
                format!("{name}:{from}-{last}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Culto { Alabanza { A, B }, Oración { C } }, Final { D }.
    fn plan() -> (Plan, [StepId; 4], [StepId; 4]) {
        let mut plan = Plan::new("Sábado");
        let culto = plan.section("Culto", None);
        let alabanza = plan.section("Alabanza", None);
        let a = plan.push("A", Action::Blank);
        let b = plan.push("B", Action::Blank);
        let oracion = plan.section("Oración", None);
        let c = plan.push("C", Action::Blank);
        let fin = plan.section("Final", None);
        let d = plan.push("D", Action::Blank);
        for (id, depth) in [(alabanza, 1), (oracion, 1)] {
            if let Some(Action::Section { depth: at, .. }) =
                plan.get_mut(id).map(|step| &mut step.action)
            {
                *at = depth;
            }
        }
        (plan, [culto, alabanza, oracion, fin], [a, b, c, d])
    }

    fn road_of(session: &Session, played: &[StepId], shut: &dyn Fn(StepId) -> bool) -> Road {
        let ahead = session.ahead(|_| Duration::from_secs(60), Duration::ZERO, 50);
        lay(
            session.plan(),
            played,
            session.current().map(|step| step.id),
            &ahead,
            shut,
            &|of| session.round_of(of),
        )
    }

    #[test]
    fn a_loop_s_next_round_is_rows_of_its_own() {
        let (mut plan, [_, alabanza, ..], [a, ..]) = plan();
        plan.loop_section(alabanza).unwrap();
        let mut session = Session::new(plan);
        session.next();
        session.next();
        let road = road_of(&session, &[a], &|_| false);
        assert_eq!(names(session.plan(), &road), "-A *B >A'1 B'1 C D");
        // One header over Alabanza's round played and on and the next,
        // inside Culto's.
        assert_eq!(
            heads(session.plan(), &road),
            "Culto0-4 Alabanza0-3 Oración4-4 Final5-5"
        );
    }

    #[test]
    fn the_road_heads_a_section_each_time_it_comes_back() {
        // A loop from B, in Alabanza, to C, in Oración.
        let (mut plan, [_, alabanza, ..], [_, b, c, _]) = plan();
        plan.loop_over(b, c).unwrap();
        let mut session = Session::new(plan);
        session.next();
        let road = road_of(&session, &[], &|_| false);
        assert_eq!(names(session.plan(), &road), "*A >B C B'1 C'1 D");
        assert_eq!(
            heads(session.plan(), &road),
            "Culto0-4 Alabanza0-1 Oración2-2 Alabanza3-3 Oración4-4 Final5-5"
        );
        // The second time is a header of its own, for its round.
        assert_eq!(road.who(Shown::Head(3)), Who::Head(alabanza, 1));
        // Each header breaks the line of the section round it: Culto's
        // has no rows of its own to run beside.
        assert_eq!(
            stretches(session.plan(), &road),
            "Alabanza:h-1 Oración:h-2 Alabanza:h-3 Oración:h-4 Final:h-5"
        );
    }

    #[test]
    fn past_the_next_round_a_loop_is_one_row() {
        // For ever: the round under way, the next, then one row.
        let (mut plan, [_, alabanza, ..], [a, ..]) = plan();
        plan.loop_section(alabanza).unwrap();
        let span = plan.section_span(alabanza).unwrap().id;
        let mut flow = plan.flow_of(lantern_engine::FlowOf::Span(span)).unwrap();
        flow.repeat = lantern_engine::Repeat::Forever;
        plan.set_flow(lantern_engine::FlowOf::Span(span), flow)
            .unwrap();
        let mut session = Session::new(plan);
        session.next();
        session.next();
        let road = road_of(&session, &[a], &|_| false);
        assert_eq!(names(session.plan(), &road), "-A *B >A'1 B'1 again2'2");
        // Five times: the same, the row saying how many more.
        let (mut plan, [_, alabanza, ..], _) = self::tests::plan();
        plan.loop_section(alabanza).unwrap();
        let span = plan.section_span(alabanza).unwrap().id;
        let mut flow = plan.flow_of(lantern_engine::FlowOf::Span(span)).unwrap();
        flow.repeat = lantern_engine::Repeat::Times(5);
        plan.set_flow(lantern_engine::FlowOf::Span(span), flow)
            .unwrap();
        let mut session = Session::new(plan);
        session.next();
        let road = road_of(&session, &[], &|_| false);
        assert_eq!(names(session.plan(), &road), "*A >B A'1 B'1 again2'2 C D");
        assert_eq!(again_words(lantern_engine::Repeat::Times(5), 2), "again ×3");
    }

    #[test]
    fn a_folded_section_is_one_thing_with_what_is_on_in_it() {
        let (plan, [_, alabanza, ..], [a, ..]) = plan();
        let mut session = Session::new(plan);
        session.next();
        session.next();
        let road = road_of(&session, &[a], &|id| id == alabanza);
        assert_eq!(names(session.plan(), &road), "*[Alabanza:B] >C D");
        // Its one row is its header, and its dot is on Culto's line.
        assert_eq!(heads(session.plan(), &road), "Culto0-1 Oración1-1 Final2-2");
        assert_eq!(road.who(Shown::Thing(0)), Who::Head(alabanza, 0));
        assert_eq!(
            stretches(session.plan(), &road),
            "Culto:h-0 Oración:h-1 Final:h-2"
        );
    }

    #[test]
    fn a_folded_section_ahead_is_next_as_one_thing() {
        let (plan, [_, _, oracion, _], _) = plan();
        let mut session = Session::new(plan);
        session.next();
        let road = road_of(&session, &[], &|id| id == oracion);
        assert_eq!(names(session.plan(), &road), "*A >B [Oración] D");
    }

    #[test]
    fn a_section_is_as_far_through_as_its_steps_whatever_their_time() {
        let min = |n: u64| Duration::from_secs(60 * n);
        // Four slides or pictures, no time of their own: a quarter each.
        let untimed = [(None, min(1)); 4];
        let played = [true, true, false, false];
        assert_eq!(through(&untimed, &played, 2, 0.0), 0.5);
        assert_eq!(through(&untimed, &played, 2, 0.5), 0.625);
        // A one-minute song and a three-minute one: the first is a
        // quarter of the time.
        let timed = [(Some(min(1)), min(1)), (Some(min(3)), min(3))];
        assert_eq!(through(&timed, &[true, false], 1, 0.0), 0.25);
        // A song of two minutes and two pictures planned at a minute
        // each: half for the song, a quarter for each picture.
        let mixed = [(Some(min(2)), min(2)), (None, min(1)), (None, min(1))];
        assert_eq!(through(&mixed, &[true, true, false], 2, 0.0), 0.75);
        // Skipped through to the last: nearly full, full at its end.
        assert_eq!(through(&untimed, &[true, true, true, false], 3, 1.0), 1.0);
    }

    #[test]
    fn rows_gone_as_a_loop_goes_round_close_up_where_they_were() {
        let (mut plan, [_, alabanza, ..], [a, b, ..]) = plan();
        plan.loop_section(alabanza).unwrap();
        let mut session = Session::new(plan);
        session.next();
        session.next();
        let whos = |road: &Road| {
            road.shown()
                .into_iter()
                .map(|row| road.who(row))
                .collect::<Vec<_>>()
        };
        let was = whos(&road_of(&session, &[a], &|_| false));
        session.next();
        let now = whos(&road_of(&session, &[], &|_| false));
        // From "-A *B >A'1 B'1 C D" to "*A >B'1 C D", under Culto's and
        // Alabanza's headers: the round played and the next one's first
        // row close up under the top one.
        let road = road_of(&session, &[], &|_| false);
        assert_eq!(names(session.plan(), &road), "*A >B'1 C D");
        assert_eq!(
            leaving(&was, &now),
            vec![
                (3, Who::Thing(What::Step(b), 0)),
                (3, Who::Thing(What::Step(a), 1))
            ]
        );
        assert_eq!(leaving(&now, &now), vec![]);
        // What is after all that stays closes up at the end.
        assert_eq!(leaving(&was, &[]).len(), was.len());
        assert!(leaving(&was, &was[..1]).iter().all(|(at, _)| *at == 1));
    }

    #[test]
    fn the_tube_fills_evenly_between_measures_and_from_empty_for_a_new_step() {
        let (_, _, [a, b, ..]) = plan();
        let start = Instant::now();
        let (on_a, on_b) = (Who::Thing(What::Step(a), 0), Who::Thing(What::Step(b), 0));
        let mut tube = Tube::default();
        // As it first shows, it is as full as it is.
        tube.towards(Some((on_a, 0.4)), start, false);
        assert_eq!(tube.shown(Some(on_a), 0.4, start), 0.4);
        assert_eq!(tube.next_look(start), None);
        // Measured again, it fills there evenly over the time till the
        // next measure.
        tube.towards(Some((on_a, 0.5)), start, false);
        let half = tube.shown(Some(on_a), 0.5, start + GLIDE / 2);
        assert!((half - 0.45).abs() < 1e-4, "{half}");
        assert!(tube.next_look(start).is_some_and(|at| at > start));
        // Something new on: its tube fills from empty.
        tube.towards(Some((on_b, 0.2)), start, false);
        assert_eq!(tube.shown(Some(on_b), 0.2, start), 0.0);
        assert_eq!(tube.next_look(start), Some(start), "every frame");
        assert_eq!(tube.shown(Some(on_b), 0.2, start + GLIDE), 0.2);
        // With nothing moving, it is where it is going at once.
        tube.towards(Some((on_a, 0.9)), start, true);
        assert_eq!(tube.shown(Some(on_a), 0.9, start), 0.9);
        assert_eq!(tube.next_look(start), None);
    }

    #[test]
    fn what_goes_on_by_itself_says_so_of_the_thing_before() {
        let (mut plan, ..) = plan();
        plan.flow.joined = true;
        let mut session = Session::new(plan);
        session.next();
        let road = road_of(&session, &[], &|_| false);
        assert!(road.things.iter().rev().skip(1).all(|thing| thing.joined));
        assert!(
            !road.things.last().unwrap().joined,
            "nothing after the last"
        );
    }
}
