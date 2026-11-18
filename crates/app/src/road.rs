//! Run's rail: the road the service takes, not the plan's shape. What
//! was played, what is on, then what comes, in the order it will come:
//! a loop's next round is rows of its own, a drawn order stands as it
//! was drawn. A folded section is one thing on it, whichever of its
//! steps is on.

use std::time::Duration;

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{Alignment, Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};
use lantern_engine::timing::{Phase, Timing};
use lantern_engine::{Action, Coming, FlowOf, Plan, Step, StepId};

use crate::look::{self, BODY, BOLD, META, Tone};
use crate::plan_pane::{RAIL, RAIL_DOT, RAIL_TOP};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// A section's bracket beside a run of things next to each other that
/// are in it. A section the road comes back to has one for each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Brace {
    pub(crate) section: StepId,
    pub(crate) first: usize,
    pub(crate) last: usize,
    /// How far out it is: 0 beside the rows, one more for each section
    /// it holds a bracket of.
    pub(crate) lane: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Road {
    pub(crate) things: Vec<Thing>,
    pub(crate) braces: Vec<Brace>,
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
        });
        rows.push(heading);
    }
    // The innermost section a thing is shown in: a folded one's own, a
    // step's deepest one; and how deep it is.
    let depth_of = |id: StepId| plan.get(id).and_then(|step| step.depth()).unwrap_or(0);
    let shown_in: Vec<Vec<StepId>> = things
        .iter()
        .zip(&rows)
        .map(|(thing, &at)| {
            let mut sections = plan.within(at);
            if let What::Folded { section, .. } = thing.what {
                sections.push(section);
            }
            sections
        })
        .collect();
    let mut braces = Vec::new();
    let mut n = 0;
    while n < things.len() {
        for section in &shown_in[n] {
            // Only where a run of it starts here.
            if n > 0 && shown_in[n - 1].contains(section) {
                continue;
            }
            let mut last = n;
            while last + 1 < things.len() && shown_in[last + 1].contains(section) {
                last += 1;
            }
            let lane = shown_in[n..=last]
                .iter()
                .filter_map(|sections| sections.last())
                .map(|inner| depth_of(*inner).saturating_sub(depth_of(*section)))
                .max()
                .unwrap_or(0);
            braces.push(Brace {
                section: *section,
                first: n,
                last,
                lane,
            });
        }
        n += 1;
    }
    Road { things, braces }
}

/// How the road is drawn beside its rows.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Solid(Color),
    Hollow(Color),
    On(Color),
    Stack { color: Color, solid: bool },
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

#[derive(Debug, Clone, PartialEq)]
struct Drawn {
    marks: Vec<Mark>,
    /// From each row to the one after it.
    links: Vec<Option<Link>>,
    braces: Vec<(Brace, Color, String, Option<String>)>,
    places: Places,
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
    /// thing a row, with its dot, the tube to what is next, and the
    /// sections' brackets.
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
        let lanes = road
            .braces
            .iter()
            .map(|brace| brace.lane + 1)
            .max()
            .unwrap_or(0);
        let wide = track::side_room(lanes, false);
        let places = self.places.clone();
        let section_of = |id: StepId| timing.sections.iter().find(|s| s.id == id);
        let mut list = column![].spacing(2);
        let mut marks = Vec::new();
        for (n, thing) in road.things.iter().enumerate() {
            let place = match thing.place {
                Place::Played => crate::plan_pane::Place::Done,
                Place::On => crate::plan_pane::Place::Current,
                Place::Next => crate::plan_pane::Place::Next,
                Place::Coming => crate::plan_pane::Place::Later,
            };
            let content: Element<'a, Message> = match thing.what {
                What::Step(id) => {
                    let Some(step) = plan.get(id) else { continue };
                    let issue = issues.iter().find(|issue| issue.step == id);
                    let noted = crate::notes::numbers_on(plan, lantern_engine::NoteOn::Step(id));
                    let mut body = column![self.step_row(step, place, issue, &noted, false)];
                    if thing.place == Place::On {
                        for (_, n, words) in
                            crate::notes::on_step(plan, id, true, &|id| engine.reached(id))
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
                What::Folded { section, holds } => {
                    self.road_folded(plan, section, holds, thing.place)
                }
                What::Again { of, from } => {
                    let words = plan
                        .flow_of(of)
                        .map_or(String::new(), |flow| again_words(flow.repeat, from));
                    container(text(words).size(BODY).color(p.on_surface_variant))
                        .padding([12, 10])
                        .into()
                }
            };
            marks.push(match (thing.what, thing.place) {
                (What::Again { .. }, _) => Mark::Stack {
                    color: p.on_surface_variant,
                    solid: false,
                },
                (What::Folded { .. }, Place::Played) => Mark::Stack {
                    color: p.outline,
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
                (What::Folded { .. }, Place::Coming) => Mark::Stack {
                    color: p.primary,
                    solid: false,
                },
                (_, Place::Played) => Mark::Solid(p.outline),
                (_, Place::On) => Mark::On(p.tertiary),
                (_, Place::Next) => Mark::Hollow(p.tertiary),
                (_, Place::Coming) if thing.joined => Mark::Solid(p.primary),
                (_, Place::Coming) => Mark::Hollow(p.on_surface_variant),
            });
            let dot = track::seen(Key::Road(n), &places, Space::new().width(RAIL).height(Fill));
            // Beside it, its innermost section's bracket: twice folds that
            // section, or opens it.
            let inner = road
                .braces
                .iter()
                .filter(|brace| (brace.first..=brace.last).contains(&n))
                .min_by_key(|brace| brace.lane)
                .map(|brace| brace.section);
            let mut room = mouse_area(Space::new().width(wide).height(Fill));
            if let Some(section) = inner {
                room = room
                    .on_double_click(Message::Fold(section))
                    .interaction(mouse::Interaction::Pointer);
            }
            let side = track::seen(Key::RoadSide(n), &places, room);
            list = list.push(
                container(row![dot, container(content).width(Fill), side].height(Length::Shrink))
                    .padding(iced::Padding::default().bottom(2.0)),
            );
        }
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
                    _ if thing.joined => Link::Plain(p.primary),
                    _ => Link::Waits(p.on_surface_variant),
                })
            })
            .collect();
        let braces = road
            .braces
            .iter()
            .filter_map(|brace| {
                let step = plan.get(brace.section)?;
                let things = &road.things[brace.first..=brace.last];
                let color = if things.iter().any(|t| t.place == Place::On) {
                    p.tertiary
                } else if things.iter().all(|t| t.place == Place::Played) {
                    p.outline
                } else {
                    p.primary
                };
                let goal = match step.action {
                    Action::Section { budget, .. } => budget.filter(|b| !b.is_zero()),
                    _ => None,
                };
                let round = things.first().map_or(0, |t| t.round);
                let sub = match (round, section_of(step.id).map(|s| s.phase)) {
                    (round, _) if round > 0 => Some(format!("round {}", round + 1)),
                    (_, Some(Phase::Done { took })) => Some(match goal {
                        Some(goal) => format!(
                            "✓ {} of {}",
                            crate::length::clock(took.unwrap_or_default()),
                            crate::length::clock(goal)
                        ),
                        None => format!("✓ {}", crate::length::clock(took.unwrap_or_default())),
                    }),
                    (_, Some(Phase::Running { elapsed, .. })) => Some(match goal {
                        Some(goal) => format!(
                            "{} of {}",
                            crate::length::clock(elapsed),
                            crate::length::clock(goal)
                        ),
                        None => crate::length::clock(elapsed),
                    }),
                    _ => Some(match goal {
                        Some(goal) => format!("◎ {}", crate::length::clock(goal)),
                        None => format!("Σ {}", crate::length::clock(self.planned(plan, step.id))),
                    }),
                };
                Some((*brace, color, step.label.clone(), sub))
            })
            .collect();
        let drawn = Drawn {
            marks,
            links,
            braces,
            places: places.clone(),
        };
        stack![list, iced::widget::canvas(drawn).width(Fill).height(Fill)]
            .width(Fill)
            .into()
    }

    /// A folded section's one row on the road: the step of it that is
    /// on, how far in, and how far in that is; else how many steps and
    /// how long. Pressed, it opens.
    fn road_folded<'a>(
        &'a self,
        plan: &Plan,
        section: StepId,
        holds: Option<StepId>,
        place: Place,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let Some(at) = plan.position(section) else {
            return Space::new().into();
        };
        let steps: Vec<&Step> = plan.steps()[at + 1..plan.group_end(at)]
            .iter()
            .filter(|step| !step.is_section())
            .collect();
        let ink = match place {
            Place::Played => p.outline,
            Place::On => p.tertiary,
            _ => p.on_surface,
        };
        let body: Element<'a, Message> = match holds.and_then(|id| plan.get(id)) {
            Some(on) => {
                let k = steps
                    .iter()
                    .position(|step| step.id == on.id)
                    .map_or(1, |k| k + 1);
                let (up, _) = self.on_for(on);
                let length = self.estimate(on).length;
                column![
                    crate::widgets::single_line(on.label.clone())
                        .size(BODY)
                        .font(BOLD)
                        .color(ink),
                    text(format!(
                        "{k} of {} · {} / {}",
                        steps.len(),
                        crate::length::clock(up),
                        crate::length::clock(length)
                    ))
                    .size(META)
                    .color(ink),
                ]
                .into()
            }
            None => row![
                text(match steps.len() {
                    1 => "1 step".to_string(),
                    n => format!("{n} steps"),
                })
                .size(BODY)
                .color(ink),
                Space::new().width(Fill),
                text(match place {
                    Place::Played => String::new(),
                    _ => crate::length::clock(self.planned(plan, section)),
                })
                .size(META)
                .color(p.on_surface_variant),
            ]
            .align_y(Alignment::Center)
            .into(),
        };
        let tone = match place {
            Place::On => Tone::Live,
            _ => Tone::Plain,
        };
        mouse_area(
            button(body)
                .width(Fill)
                .padding([10, 10])
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

/// A dot's radius, a ring's room round the one that is on, a tube's
/// width, and a brace's curve.
const DOT: f32 = 5.0;
const RING: f32 = 4.0;
const TUBE: f32 = 8.0;
const CURVE: f32 = 7.0;

impl canvas::Program<Message> for Drawn {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let places = self.places.borrow().clone();
        let at = |key: Key| {
            places.get(&key).map(|place| Rectangle {
                x: place.x - bounds.x,
                y: place.y - bounds.y,
                ..*place
            })
        };
        let middle = |n: usize| {
            at(Key::Road(n))
                .map(|rail| Point::new(rail.center_x(), rail.y + RAIL_TOP + RAIL_DOT / 2.0))
        };
        let reach = |mark: Mark| match mark {
            Mark::On(_) => DOT + RING,
            _ => DOT,
        };
        let mut frame = Frame::new(renderer, bounds.size());
        // The links first, under the dots.
        for (n, link) in self.links.iter().enumerate() {
            let (Some(link), Some(from), Some(to)) = (link, middle(n), middle(n + 1)) else {
                continue;
            };
            let top = from.y + reach(self.marks[n]) + 2.0;
            let foot = to.y - reach(self.marks[n + 1]) - 2.0;
            if foot <= top {
                continue;
            }
            match *link {
                Link::Plain(color) => frame.stroke(
                    &Path::line(Point::new(from.x, top), Point::new(from.x, foot)),
                    Stroke::default().with_width(2.0).with_color(color),
                ),
                Link::Tube { full, color, waits } => {
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
                    if waits && full >= 1.0 {
                        pause(
                            &mut frame,
                            Point::new(from.x + 12.0, (top + foot) / 2.0),
                            color,
                        );
                    }
                }
                Link::Waits(color) => {
                    pause(&mut frame, Point::new(from.x, (top + foot) / 2.0), color)
                }
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
                Mark::On(color) => {
                    frame.stroke(
                        &Path::circle(at, DOT + RING - 0.75),
                        Stroke::default()
                            .with_width(1.5)
                            .with_color(Color { a: 0.55, ..color }),
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
        for (brace, color, name, sub) in &self.braces {
            let (Some(first), Some(last)) = (
                at(Key::RoadSide(brace.first)),
                at(Key::RoadSide(brace.last)),
            ) else {
                continue;
            };
            let x = first.x + track::brace_at(brace.lane) + 3.0;
            let (top, foot) = (first.y + 2.0, last.y + last.height - 4.0);
            if foot - top < 12.0 {
                continue;
            }
            let middle = (top + foot) / 2.0;
            let round = CURVE.min((foot - top) / 4.0);
            let shape = Path::new(|path| {
                path.move_to(Point::new(x, top));
                path.quadratic_curve_to(
                    Point::new(x + round, top),
                    Point::new(x + round, top + round),
                );
                path.line_to(Point::new(x + round, middle - round));
                path.quadratic_curve_to(
                    Point::new(x + round, middle),
                    Point::new(x + round + 6.0, middle),
                );
                path.quadratic_curve_to(
                    Point::new(x + round, middle),
                    Point::new(x + round, middle + round),
                );
                path.line_to(Point::new(x + round, foot - round));
                path.quadratic_curve_to(Point::new(x + round, foot), Point::new(x, foot));
            });
            frame.stroke(&shape, Stroke::default().with_width(1.6).with_color(*color));
            // Only the innermost write beside their brace, as on the
            // plan's rail: there is room for one.
            if brace.lane > 0 {
                continue;
            }
            let left = x + track::BRACE;
            let lines = usize::from(sub.is_some()) + 1;
            let mut y = middle - (lines as f32 - 1.0) * 7.5;
            frame.fill_text(canvas::Text {
                content: name.clone(),
                position: Point::new(left, y),
                color: *color,
                size: 11.5.into(),
                font: BOLD,
                align_y: iced::alignment::Vertical::Center,
                ..canvas::Text::default()
            });
            if let Some(sub) = sub {
                y += 15.0;
                frame.fill_text(canvas::Text {
                    content: sub.clone(),
                    position: Point::new(left, y),
                    color: Color { a: 0.85, ..*color },
                    size: 10.5.into(),
                    align_y: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
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

    fn braces(plan: &Plan, road: &Road) -> String {
        road.braces
            .iter()
            .map(|brace| {
                let name = &plan.get(brace.section).unwrap().label;
                format!("{name}{}-{}@{}", brace.first, brace.last, brace.lane)
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
        // Alabanza twice: the round played and on, then the next; Culto
        // round all of it, a lane further out.
        assert_eq!(
            braces(session.plan(), &road),
            "Culto0-4@1 Alabanza0-3@0 Oración4-4@0 Final5-5@0"
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
        assert_eq!(
            braces(session.plan(), &road),
            "Culto0-1@1 Alabanza0-0@0 Oración1-1@0 Final2-2@0"
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
