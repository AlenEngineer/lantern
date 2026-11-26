//! The left pane: the plan. Its steps, grouped under section headers that
//! say how each part is keeping to its time, edited in place (see
//! [`crate::plan_edit`]), and behind the plan's name the saved plans.
//!
//! Clicking a step selects it, opens its edit strip and lines it up in the
//! preview; Enter puts it on stage, and Ctrl+Enter adds after it.

use std::time::{Duration, SystemTime};

use iced::widget::{
    Space, button, column, container, mouse_area, row, stack, text, text_input, tooltip,
};
use iced::{Alignment, Element, Fill, Length, Rectangle, mouse};
use lantern_engine::{
    Action, Basis, Estimate, FadeTime, Flow, FlowOf, Kind, Location, Media, Pace, Phase, Plan,
    Query, Repeat, Resource, SectionTiming, Step, StepId, TimeOfDay, Timing,
};

use crate::dashboard::{NAME_BOX, level_tone, op, unit};
use crate::drag::Half;
use crate::keys::{self, Op};
use crate::length;
use crate::look::{self, BADGE, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL_SMALL, TITLE, Tone};
use crate::palette::Palette;
use crate::pixels::{Pixels, looks_soft};
use crate::plan_edit::{NewStep, Part, goes_on_when, insertion_mark};
use crate::plans::{self, Entry, Notice, Switch, file_name};
use crate::ready::{Issue, Level, Reach};
use crate::sources::CardAt;
use crate::widgets::{self, Filled, hinted, pill, scrolling, single_line, wide};
use crate::{App, Clicked, FadeEnd, Message, track};

/// The id of the pane's rows, to scroll them.
pub const ROWS: &str = "plan-rows";

/// How far in a section's header sits for each section round it, and the
/// gap for what is carried for each level in it would land.
pub(crate) const INDENT: f32 = 12.0;

/// The field the plan's target time is typed in.
pub(crate) const TARGET_BOX: &str = "plan-target";
/// The field the plan's start time is typed in.
pub(crate) const STARTS_BOX: &str = "plan-starts";

/// The rail beside the steps: how wide its column is, its dots, its
/// line, and how far down a row of one line its dot sits, level with the
/// step's name.
pub(crate) const RAIL: f32 = 20.0;
pub(crate) const RAIL_DOT: f32 = 10.0;
const RAIL_LINE: f32 = 2.0;
pub(crate) const RAIL_TOP: f32 = 15.0;
/// How far the ring round the dot of the step that is up stands off it.
const RAIL_RING: f32 = 4.0;

/// About how tall a step's row is, and each line under it, and the gap a
/// drag opens: for what is cut to a height on its way open or closed, and
/// a row not drawn yet. A little over is right; under would cut a row
/// that should show whole.
const ROW_TALL: f32 = 50.0;
const PART_TALL: f32 = 22.0;
const GAP_TALL: f32 = 48.0;
/// How many places for carried steps are drawn at most.
const PLACES: usize = 6;

/// A step's picture while planning, 16:9, and the room above and below
/// it: together, how tall the first line of its row is.
const PICTURE: f32 = 64.0;
const PICTURE_ROOM: f32 = 5.0;
const TALL_LINE: f32 = PICTURE * 9.0 / 16.0 + 2.0 * PICTURE_ROOM;
/// How far a picture's badges sit in from its corners, and the room
/// either side of a badge's words.
const BADGE_INSET: f32 = 2.0;
const BADGE_PAD: f32 = 3.0;
/// How wide the column of the steps' times of day is.
const TIME_WIDE: f32 = 40.0;
/// How large a section's name is on its header, a little over its
/// steps' names, how wide the gauge of its time is, and how tall the
/// header's line is.
const NAME: f32 = 14.0;
const GAUGE: f32 = 64.0;
pub(crate) const HEAD_LINE: f32 = 26.0;

/// How the plan's steps are laid out. While planning a step's row is
/// tall: its time of day, its dot, its picture, its name over a line of
/// what it is, and its length. In a run, where the plan is a narrow pane
/// beside the stage, it is one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rows {
    Tall,
    Line,
}

impl Rows {
    fn of(mode: crate::mode::Mode) -> Rows {
        match mode {
            crate::mode::Mode::Plan => Rows::Tall,
            crate::mode::Mode::Run | crate::mode::Mode::Live => Rows::Line,
        }
    }

    /// How far down a step's row its dot sits: level with the middle of
    /// its picture, or with its name.
    fn dot(self) -> f32 {
        match self {
            Rows::Tall => (TALL_LINE - RAIL_DOT) / 2.0,
            Rows::Line => RAIL_TOP,
        }
    }

    /// How tall the place for one carried step is: about as tall as a
    /// step's row.
    fn place(self) -> f32 {
        match self {
            Rows::Tall => TALL_LINE,
            Rows::Line => 36.0,
        }
    }

    /// How far down a place, or a folded section's row, its dot sits: in
    /// a place's middle.
    fn place_dot(self) -> f32 {
        (self.place() - RAIL_DOT) / 2.0
    }

    /// The room over a section's header, where a new group starts.
    fn head_room(self) -> f32 {
        match self {
            Rows::Tall => 14.0,
            Rows::Line => 8.0,
        }
    }

    /// The room round what is in a step's row, above and below it and to
    /// either side.
    fn padding(self) -> [f32; 2] {
        match self {
            Rows::Tall => [PICTURE_ROOM, 10.0],
            Rows::Line => [10.0, 10.0],
        }
    }

    /// How far in from the start of a step's row its name is: past its
    /// picture, with the row's room between.
    fn inset(self) -> f32 {
        match self {
            Rows::Tall => PICTURE + 8.0,
            Rows::Line => 0.0,
        }
    }

    /// About how far left of the end of a step's row its notes' marks
    /// are: past its length, and while planning the mark beside it.
    fn marks_from_end(self) -> f32 {
        match self {
            Rows::Tall => 63.0,
            Rows::Line => 44.0,
        }
    }
}

/// What the place of a file carried in from the library goes by, among the
/// places of steps: no step of a plan has it.
fn held_place(n: usize) -> StepId {
    serde_json::from_value(serde_json::Value::from(u64::MAX - n as u64))
        .expect("a step id is a number")
}

/// What a row of the plan's list is, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listed {
    /// A section's header, `depth` in. Folded, it stands for all it
    /// holds.
    Head {
        section: StepId,
        depth: u8,
        folded: bool,
    },
    Step(StepId),
    /// The place of a section with nothing in it, or nothing but what is
    /// carried: where its steps will be.
    Empty(StepId),
    /// Where what is carried would land.
    Gap,
    /// The room under a section's last row, where its foot is pulled.
    Foot(StepId),
}

/// The rows of `plan` as its list shows them: each section's header
/// before what it holds, one `shut` as its header alone but for a step
/// `kept` in sight. What is `carried` is out, and the gap opens where it
/// would land: before the row `landing` says among the rest, as far in
/// as it says, so inside each section further out and after the feet of
/// the others. With `feet`, each section has one under its last row.
fn listed(
    plan: &Plan,
    shut: &dyn Fn(StepId) -> bool,
    kept: &[StepId],
    carried: &[StepId],
    landing: Option<(usize, u8)>,
    feet: bool,
) -> Vec<Listed> {
    let steps = plan.steps();
    let mut rows = Vec::new();
    // The sections whose feet are still to come, innermost last: where
    // what they hold ends, and how far in they are.
    let mut open: Vec<(StepId, usize, u8)> = Vec::new();
    let mut hidden_to = 0;
    let mut rest = 0;
    for index in 0..=steps.len() {
        let step = steps.get(index);
        if step.is_some_and(|step| carried.contains(&step.id)) {
            continue;
        }
        let gap = landing
            .filter(|(at, _)| *at == rest)
            .map(|(_, level)| level);
        let mut ends = |rows: &mut Vec<Listed>, inside: Option<u8>| {
            while let Some(&(id, end, depth)) = open.last() {
                if end > index || inside.is_some_and(|level| depth < level) {
                    break;
                }
                open.pop();
                if feet {
                    rows.push(Listed::Foot(id));
                }
            }
        };
        ends(&mut rows, gap);
        if gap.is_some() {
            rows.push(Listed::Gap);
        }
        ends(&mut rows, None);
        let Some(step) = step else {
            break;
        };
        rest += 1;
        if index < hidden_to && (step.is_section() || !kept.contains(&step.id)) {
            continue;
        }
        let Some(depth) = step.depth() else {
            rows.push(Listed::Step(step.id));
            continue;
        };
        let end = plan.group_end(index);
        let folded = shut(step.id);
        rows.push(Listed::Head {
            section: step.id,
            depth,
            folded,
        });
        if folded {
            hidden_to = end;
            continue;
        }
        // Landing in a section with nothing in it, the gap is its place.
        let lands_in = landing.is_some_and(|(at, level)| at == rest && level > depth);
        if (index + 1..end).all(|at| carried.contains(&steps[at].id)) && !lands_in {
            rows.push(Listed::Empty(step.id));
        }
        open.push((step.id, end, depth));
    }
    rows
}

/// How the line of the section `id` stands in `plan` while `lit` is the
/// one pointed at or taking in what is carried: that one lit, those
/// inside it as they are, the rest stepping back.
fn lit_of(plan: &Plan, lit: Option<StepId>, id: StepId) -> track::Light {
    match lit {
        None => track::Light::Plain,
        Some(lit) if lit == id => track::Light::Lit,
        Some(lit)
            if plan
                .position(id)
                .is_some_and(|at| plan.within(at).contains(&lit)) =>
        {
            track::Light::Within
        }
        Some(_) => track::Light::Back,
    }
}

/// The sections a row of the list is in, the outermost first. A
/// section's header, place and foot are in it too, and the gap is in
/// `gap_in`, the sections round where what is carried would land,
/// innermost first.
fn inside(plan: &Plan, row: Listed, gap_in: &[StepId]) -> Vec<StepId> {
    let within = |id: StepId| {
        plan.position(id)
            .map_or_else(Vec::new, |at| plan.within(at))
    };
    match row {
        Listed::Step(id) => within(id),
        Listed::Head { section, .. } | Listed::Empty(section) | Listed::Foot(section) => {
            let mut round = within(section);
            round.push(section);
            round
        }
        Listed::Gap => gap_in.iter().rev().copied().collect(),
    }
}

/// Each section's line beside `rows`, in pieces, top to bottom: the
/// section, and the rows a piece starts and ends at. A piece starts at
/// the section's header, or at the first of its rows with a dot after a
/// section inside it ended, and ends at the last of those before the next
/// header inside it, or at its foot. A folded section is a row with a
/// dot of the one round it, as the gap is of the innermost of `gap_in`.
fn stretches(plan: &Plan, rows: &[Listed], gap_in: &[StepId]) -> Vec<(StepId, usize, usize)> {
    let mut pieces = Vec::new();
    // The sections open so far: where their piece starts, and the last
    // of its rows with a dot.
    let mut open: Vec<(StepId, Option<usize>, Option<usize>)> = Vec::new();
    let round = |id: StepId| {
        plan.position(id)
            .and_then(|at| plan.within(at).last().copied())
    };
    for (n, row) in rows.iter().enumerate() {
        let dotted = match *row {
            Listed::Head {
                section,
                folded: false,
                ..
            } => {
                if let Some(piece) =
                    round(section).and_then(|round| open.iter_mut().find(|(id, ..)| *id == round))
                    && let (Some(from), Some(to)) = (piece.1.take(), piece.2.take())
                {
                    pieces.push((piece.0, from, to));
                }
                open.push((section, Some(n), None));
                continue;
            }
            Listed::Foot(section) => {
                if let Some(at) = open.iter().position(|(id, ..)| *id == section)
                    && let (_, Some(from), _) = open.remove(at)
                {
                    pieces.push((section, from, n));
                }
                continue;
            }
            Listed::Empty(_) => continue,
            Listed::Step(id) | Listed::Head { section: id, .. } => round(id),
            Listed::Gap => gap_in.first().copied(),
        };
        if let Some(piece) =
            dotted.and_then(|holder| open.iter_mut().find(|(id, ..)| *id == holder))
        {
            piece.1.get_or_insert(n);
            piece.2 = Some(n);
        }
    }
    pieces.extend(
        open.into_iter()
            .filter_map(|(section, from, to)| Some((section, from?, to?))),
    );
    pieces.sort_by_key(|(_, from, _)| *from);
    pieces
}

/// The last of `rows` in `section`, but for its foot: where its band
/// ends.
fn last_in(plan: &Plan, rows: &[Listed], section: StepId, gap_in: &[StepId]) -> Option<usize> {
    rows.iter().rposition(|row| {
        !matches!(row, Listed::Foot(_)) && inside(plan, *row, gap_in).contains(&section)
    })
}

/// What a plan takes: see [`App::plan_sums`].
struct PlanSums {
    filled: Duration,
    total: Duration,
    open: bool,
}

impl PlanSums {
    /// What the plan is held against: its own goal, or else what its
    /// sections' times come to, where they give it more than its steps.
    fn against(&self, target: Option<Duration>) -> Option<Duration> {
        target.or((self.total != self.filled).then_some(self.total))
    }

    /// What its steps come to, "Σ 7:48": at least that, "Σ 7:48+", where
    /// something comes again with no end.
    fn said(&self) -> String {
        match self.open {
            true => format!("Σ {}+", length::clock(self.filled)),
            false => format!("Σ {}", length::clock(self.filled)),
        }
    }

    /// When a plan that `starts` then should end: after its goal, or else
    /// after what its steps and its sections' times come to.
    fn ends(&self, starts: Option<TimeOfDay>, target: Option<Duration>) -> Option<TimeOfDay> {
        starts.map(|starts| starts.plus(target.unwrap_or(self.total)))
    }
}

/// What the plan's title says of it while planning, in order, each with
/// whether it is over: how many `steps` it has; what they come to, over
/// where that is more than what it is held against; that, its goal
/// ([`PlanSums::against`]); and the time of day it `starts` and so ends.
/// Only what is known is said.
fn plan_facts(
    steps: usize,
    target: Option<Duration>,
    starts: Option<TimeOfDay>,
    sums: &PlanSums,
) -> Vec<(String, bool)> {
    let against = sums.against(target);
    let over = against.is_some_and(|against| sums.filled > against);
    let mut facts = vec![(count(steps, "step"), false), (sums.said(), over)];
    facts.extend(against.map(|against| (format!("◎ {}", length::clock(against)), false)));
    if let (Some(starts), Some(ends)) = (starts, sums.ends(starts, target)) {
        facts.push((format!("starts {starts}, ends {ends}"), false));
    }
    facts
}

impl App {
    /// The plan's head, over its rows, then its sections' parts. While
    /// planning it is the page's title: its name, large, and what is
    /// known of it ([`plan_facts`]). In a run, where the strip has its
    /// name, it is its name small and the time it is meant to take.
    fn plan_head(&self, plan: &Plan, timing: &Timing) -> Element<'_, Message> {
        let p = self.palette;
        let sums = self.plan_sums(plan);
        let (line, apart): (Element<'_, Message>, f32) = match self.mode {
            crate::mode::Mode::Plan => {
                let steps = plan.steps().iter().filter(|s| !s.is_section()).count();
                let facts = plan_facts(steps, plan.target, plan.starts, &sums);
                let mut said = row![].spacing(5).align_y(Alignment::Center);
                for (at, (words, over)) in facts.into_iter().enumerate() {
                    if at > 0 {
                        said = said.push(single_line("·").size(META).color(p.on_surface_variant));
                    }
                    let ink = if over { p.error } else { p.on_surface_variant };
                    said = said.push(single_line(words).size(META).color(ink));
                }
                let title = row![
                    single_line(plan.title.clone())
                        .size(TITLE)
                        .font(BOLD)
                        .color(p.on_surface),
                    container(said)
                        .width(Fill)
                        .clip(true)
                        .padding(iced::Padding::default().bottom(4.0)),
                ]
                .spacing(12)
                .align_y(Alignment::End);
                (track::seen(track::Key::Title, &self.places, title), 14.0)
            }
            _ => {
                let mut line = row![
                    Space::new().width(Fill),
                    text(plan.title.clone())
                        .size(META)
                        .font(BOLD)
                        .color(p.primary)
                ]
                .spacing(10)
                .align_y(Alignment::Center);
                // With no goal of its own, what its sections' times come to.
                if let Some(against) = sums.against(plan.target) {
                    line = line.push(
                        text(format!("◎ {}", length::clock(against)))
                            .size(META)
                            .color(p.on_surface_variant),
                    );
                }
                (line.into(), 4.0)
            }
        };
        let mut head = column![line].spacing(apart);
        if let Some(parts) = self.section_parts(plan, timing) {
            head = head.push(parts);
        }
        container(head).padding([2, 8]).into()
    }

    /// Where `aim` is on the dashboard, as last laid out: for a script's
    /// pointer. A spot of a row is across its middle; anything else is
    /// its middle.
    pub(crate) fn place_of(&self, aim: crate::script::Aim) -> Option<iced::Point> {
        use crate::script::{Aim, Spot};
        let places = self.places.borrow();
        let middle = |key: track::Key, scrolled: f32| {
            let seen = places.get(&key)?;
            Some(iced::Point::new(
                seen.center_x(),
                seen.center_y() - scrolled,
            ))
        };
        let (entry, spot) = match aim {
            Aim::Row(entry, spot) => (entry, spot),
            Aim::Title => return middle(track::Key::Title, self.plan_scrolled),
            Aim::RingNote => return middle(track::Key::RingNote, 0.0),
            Aim::Library(n) => {
                // The library's list is told nothing once its rows fit in
                // it, as a layout can make them: then it is at its top,
                // whatever it was last told.
                let tall = |key| places.get(&key).map(|seen: &Rectangle| seen.height);
                let most = tall(track::Key::ShelfRows)
                    .zip(tall(track::Key::ShelfView))
                    .map_or(0.0, |(rows, view)| (rows - view).max(0.0));
                return middle(track::Key::Shelf(n), self.shelf_scrolled.min(most));
            }
        };
        let id = self.engine.0.plan().steps().get(entry)?.id;
        let key = match spot {
            Spot::Chevron => return middle(track::Key::Chevron(id), self.plan_scrolled),
            Spot::Foot => track::Key::Foot(id),
            Spot::Top | Spot::Middle | Spot::Bottom => track::Key::Whole(id),
        };
        let row = *places.get(&key)?;
        let y = match spot {
            Spot::Top => row.y + FOOT / 2.0,
            Spot::Bottom => row.y + row.height * 0.75,
            Spot::Middle | Spot::Foot | Spot::Chevron => row.center_y(),
        };
        Some(iced::Point::new(row.center_x(), y - self.plan_scrolled))
    }
}

impl App {
    /// A part for each of the plan's own sections, solid in its colour,
    /// as wide as the time it is given (or else takes); past its time,
    /// the share of its steps that is over is in the colour of what is
    /// wrong. How full it is against its time is its header's gauge. One
    /// that holds nothing is faded. Under each, its name in its colour
    /// and what its steps take, or for one that holds nothing its name
    /// and "empty", as much of that as its width has room for
    /// ([`words_said`]). `None` with no sections.
    fn section_parts<'a>(&self, plan: &Plan, timing: &Timing) -> Option<Element<'a, Message>> {
        let p = self.palette;
        let colors = plan.section_colors();
        let parts: Vec<SectionPart> = timing
            .sections
            .iter()
            .filter(|section| {
                plan.get(section.id)
                    .is_some_and(|step| step.depth() == Some(0))
            })
            .map(|section| {
                let planned = match section.phase {
                    Phase::Coming { planned } => planned,
                    Phase::Running { expected, .. } => expected,
                    Phase::Done { took } => took.unwrap_or_default(),
                };
                let hue = section_hue(p, &colors, section.id);
                let empty = plan
                    .position(section.id)
                    .is_some_and(|at| plan.group_end(at) == at + 1);
                let name = plan
                    .get(section.id)
                    .map_or_else(String::new, |step| step.label.clone());
                let (colour, over, words) = if empty {
                    let faded = |ink| Palette::tint(ink, EMPTY_FADED);
                    (
                        faded(hue),
                        0.0,
                        vec![
                            (name, faded(hue), BOLD),
                            ("· empty".to_string(), faded(p.on_surface_variant), look::UI),
                        ],
                    )
                } else {
                    let over = over_share(section.budget, planned);
                    (
                        hue,
                        over,
                        vec![
                            (name, hue, BOLD),
                            (
                                length::clock(planned),
                                if over > 0.0 {
                                    p.error
                                } else {
                                    p.on_surface_variant
                                },
                                look::UI,
                            ),
                        ],
                    )
                };
                SectionPart {
                    wide: section.budget.unwrap_or(planned).as_secs().clamp(1, 60_000) as u16,
                    colour,
                    over,
                    words,
                }
            })
            .collect();
        if parts.is_empty() {
            return None;
        }
        // How wide each part is shows only once they are laid out.
        let whole: f32 = parts.iter().map(|part| f32::from(part.wide)).sum();
        let error = p.error;
        let ribbon = iced::widget::responsive(move |room| {
            let room = room.width - PART_GAP * (parts.len() - 1) as f32;
            row(parts.iter().map(|part| {
                let words: Vec<&str> = part.words.iter().map(|(word, ..)| word.as_str()).collect();
                let said = words_said(room * f32::from(part.wide) / whole, &words);
                let label = row(part.words.iter().take(said).map(|(word, ink, font)| {
                    single_line(word.clone())
                        .size(META)
                        .font(*font)
                        .color(*ink)
                        .into()
                }))
                .spacing(WORD_GAP);
                column![
                    ribbon_bar(part.colour, part.over, error),
                    container(label).width(Fill).clip(true),
                ]
                .spacing(5)
                .width(Length::FillPortion(part.wide))
                .into()
            }))
            .spacing(PART_GAP)
            .into()
        })
        .height(Length::Shrink);
        Some(ribbon.into())
    }
}

/// A section's part of the plan's head: its share of the width, its
/// bar's colour and the share of it that is over its time, and the words
/// under it, each in its colour and weight.
struct SectionPart {
    wide: u16,
    colour: iced::Color,
    over: f32,
    words: Vec<(String, iced::Color, iced::Font)>,
}

/// How strong a section that holds nothing is drawn in the plan's head.
const EMPTY_FADED: f32 = 0.45;

/// A section's bar in the plan's head: solid in `colour`, rounded, and
/// its last `over` of it in `error`, apart by a notch: a section's own
/// colour can be near the colour of what is wrong.
fn ribbon_bar<'a>(colour: iced::Color, over: f32, error: iced::Color) -> Element<'a, Message> {
    const TALL: f32 = 6.0;
    let piece = |ink: iced::Color, share: f32, radius: iced::border::Radius| {
        container(Space::new())
            .width(Length::FillPortion((share * 1000.0).round().max(1.0) as u16))
            .height(TALL)
            .style(move |_| container::Style {
                background: Some(ink.into()),
                border: iced::Border {
                    radius,
                    ..iced::Border::default()
                },
                ..container::Style::default()
            })
    };
    let end = TALL / 2.0;
    match over > 0.0 {
        true => row![
            piece(colour, 1.0 - over, iced::border::left(end)),
            piece(error, over, iced::border::right(end)),
        ]
        .spacing(2)
        .into(),
        false => piece(colour, 1.0, end.into()).width(Fill).into(),
    }
}

/// The room between two parts of the plan's head, and between two words
/// under one.
const PART_GAP: f32 = 3.0;
const WORD_GAP: f32 = 5.0;

/// How many of `words` a part `wide` says under it: all of them where
/// they fit, else the first, cut where it must be, and none where the
/// part is too narrow for a couple of letters and the "…" after them.
fn words_said(wide: f32, words: &[&str]) -> usize {
    const READABLE: f32 = 28.0;
    let all = words
        .iter()
        .map(|word| widgets::steady(word, META))
        .sum::<f32>()
        + WORD_GAP * words.len().saturating_sub(1) as f32;
    match wide {
        wide if wide < READABLE => 0,
        wide if all <= wide => words.len(),
        _ => words.len().min(1),
    }
}

/// What a section's header says of its time: what it has `taken` (or will
/// take), against the `budget` it is given, and the time of day it should
/// be over `by`, as "0:24 of 5:00 · by 19:15". One that holds nothing is
/// "empty", and one that is over in a service says it is `done`.
fn time_words(
    taken: Duration,
    budget: Option<Duration>,
    by: Option<TimeOfDay>,
    empty: bool,
    done: bool,
) -> String {
    let mut words = vec![match (empty, budget) {
        (true, _) => "empty".to_string(),
        (false, Some(budget)) => format!("{} of {}", length::clock(taken), length::clock(budget)),
        (false, None) => length::clock(taken),
    }];
    if empty && let Some(budget) = budget {
        words.push(length::clock(budget));
    }
    words.extend(by.map(|by| format!("by {by}")));
    if done {
        words.push("done".to_string());
    }
    words.join(" · ")
}

/// How much of what a section's steps take is past its time, from 0 to 1.
fn over_share(budget: Option<Duration>, planned: Duration) -> f32 {
    match budget {
        Some(budget) if planned > budget => {
            (planned - budget).as_secs_f32() / planned.as_secs_f32()
        }
        _ => 0.0,
    }
}

/// How full a section's gauge is: its steps against its time, full where
/// it has none; and whether they run over it.
fn part_fill(budget: Option<Duration>, planned: Duration) -> (f32, bool) {
    match budget {
        Some(budget) if planned > budget => (1.0, true),
        Some(budget) if !budget.is_zero() => (planned.as_secs_f32() / budget.as_secs_f32(), false),
        _ => (1.0, false),
    }
}

impl App {
    /// What a plan's steps come to, as the list of plans says it.
    fn takes(&self, plan: &Plan) -> String {
        self.plan_sums(plan).said()
    }

    /// A saved plan's line in the list: how many steps, what they come
    /// to, and `when` it was saved.
    fn listed_detail(&self, entry: &Entry, when: String) -> String {
        match &entry.summary {
            Ok(summary) => format!(
                "{} · {} · {when}",
                count(summary.steps, "step"),
                self.takes(&summary.plan)
            ),
            Err(_) => when,
        }
    }
}

impl App {
    /// How tall the rows of `steps` were last drawn, together; one never
    /// drawn as tall as a row usually is.
    pub(crate) fn rows_tall(&self, steps: &[StepId]) -> f32 {
        let places = self.places.borrow();
        steps
            .iter()
            .map(|id| {
                places
                    .get(&track::Key::Whole(*id))
                    .map_or(ROW_TALL, |row| row.height)
            })
            .sum()
    }
}

/// The colour of a sound that keeps going under the steps after its own.
pub(crate) const SOUND_LINE: iced::Color = iced::Color::from_rgb(0.557, 0.773, 1.0);

/// How wide the plan's goal is in its settings, pressed or typed.
const GOAL_WIDE: f32 = 54.0;

/// How tall a section's head and foot are, to be pulled: thin bands, the
/// top of its header and the room under its last row, so the rest of its
/// header, its name too, picks the section up.
const FOOT: f32 = 8.0;

/// A span as the rail shows it: what it is, the steps it runs from and
/// to, how far out from the dots it is drawn (the more of its kind it
/// holds, the further), and the section it is attached to, if it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Laid {
    of: FlowOf,
    first: usize,
    last: usize,
    lane: u8,
    section: Option<StepId>,
}

/// The spans of one kind (`wanted` says which: those that come again,
/// or those in a drawn order), and the plan where it is of that kind,
/// each in its lane.
fn laid(plan: &Plan, wanted: impl Fn(Flow) -> bool) -> Vec<Laid> {
    let steps = plan.steps();
    let step = |at: &usize| !steps[*at].is_section();
    let all = (0..steps.len())
        .find(step)
        .zip((0..steps.len()).rfind(step))
        .filter(|_| wanted(plan.flow))
        .map(|(first, last)| Laid {
            of: FlowOf::Plan,
            first,
            last,
            lane: 0,
            section: None,
        });
    // Outermost first: the plan, then the spans as the plan gives them.
    let mut found: Vec<Laid> = all
        .into_iter()
        .chain(
            plan.laid_spans()
                .into_iter()
                .filter(|(span, ..)| wanted(span.flow))
                .map(|(span, first, last)| Laid {
                    of: FlowOf::Span(span.id),
                    first,
                    last,
                    lane: 0,
                    section: match span.over {
                        lantern_engine::Over::Section(id) => Some(id),
                        lantern_engine::Over::Steps { .. } => None,
                    },
                }),
        )
        .collect();
    // From the innermost out: each one lane further than what it holds.
    for at in (0..found.len()).rev() {
        let (outer, inner) = found.split_at_mut(at + 1);
        let this = &mut outer[at];
        this.lane = inner
            .iter()
            .filter(|held| this.first <= held.first && held.last <= this.last)
            .map(|held| held.lane + 1)
            .max()
            .unwrap_or(0);
    }
    found
}

/// The room either side of the dots, the same down the whole plan so
/// the dots stay in one line: on the left for the lines of what comes
/// again, on the right for those of a drawn order, a lane for each one
/// that holds another. While planning there is always one of each,
/// since a pull can draw either.
/// Left of all of it, the steps' times of day, where they have them.
/// The rows say how far down them the dots are.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lanes<'a> {
    time: f32,
    curl: f32,
    drawn: f32,
    loops: &'a [Laid],
    orders: &'a [Laid],
    /// How many lanes each side has room for.
    loop_lanes: u8,
    order_lanes: u8,
    rows: Rows,
}

impl Lanes<'_> {
    /// All the room left of the dots' column.
    fn left(&self) -> f32 {
        self.time + self.curl
    }
}

/// `row` with the room there is between two rows of the plan under it.
fn apart<'a>(row: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(row)
        .padding(iced::Padding::default().bottom(2.0))
        .into()
}

/// The room under a section's last row: pressed and pulled, its foot
/// ends the section somewhere else.
fn foot_strip<'a>(section: StepId, places: &track::Places) -> Element<'a, Message> {
    track::seen(
        track::Key::Foot(section),
        places,
        mouse_area(Space::new().width(Fill).height(FOOT))
            .on_press(Message::PressFoot(section))
            .interaction(mouse::Interaction::ResizingVertically),
    )
}

/// Where a step is in the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Done,
    Current,
    Next,
    Later,
}

/// What a step's row says of it while planning, worked out once from the
/// step and what is known of what it shows (see [`RowFacts::of`]).
#[derive(Debug, PartialEq)]
pub(crate) struct RowFacts<'a> {
    /// The time of day it comes, where the plan has a start.
    at: Option<TimeOfDay>,
    thumb: Thumb<'a>,
    under: Under,
    /// It goes on by itself when it is done; else it waits.
    goes_on: bool,
}

/// A row's small picture: what it is of, with its size in the lower
/// right corner and its state in the upper left. A step's row and a file's
/// row in the library draw theirs alike (see [`App::row_picture`]).
#[derive(Debug, PartialEq)]
pub(crate) struct Thumb<'a> {
    picture: Picture<'a>,
    /// How long it plays, how many pages it has, a picture's pixels.
    pub(crate) size: Option<String>,
    pub(crate) state: Option<State>,
}

impl<'a> Thumb<'a> {
    /// The picture of `resource`, with what is known of it.
    pub(crate) fn of(resource: &'a Resource, known: Known) -> Thumb<'a> {
        Thumb {
            picture: Picture::Of(resource),
            size: size_words(resource, known),
            state: match (known.download, known.reach) {
                (Some(part), _) => Some(State::Coming(part)),
                (None, Reach::Stream) => Some(State::Live),
                (None, Reach::Online) => Some(State::Online),
                (None, Reach::Here) => known.soft_on().map(State::Soft),
            },
        }
    }

    /// Only `icon`: for what shows nothing of its own, or isn't looked
    /// at yet.
    pub(crate) fn kind(icon: Icon) -> Thumb<'a> {
        Thumb {
            picture: Picture::Kind(icon),
            size: None,
            state: None,
        }
    }
}

/// What a row's picture is of.
#[derive(Debug, PartialEq)]
enum Picture<'a> {
    Of(&'a Resource),
    /// The kind of thing it is, for one that shows nothing of its own.
    Kind(Icon),
}

/// What a row's picture says in its upper left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum State {
    /// On stage now, or a stream that needs the internet all the while
    /// it shows.
    Live,
    /// It comes from the internet, not downloaded yet.
    Online,
    /// How far its download has come, from 0 to 1.
    Coming(f32),
    /// It will look soft on the stage's screen, of this size.
    Soft(Pixels),
}

impl State {
    pub(crate) fn words(self) -> String {
        match self {
            State::Live => "● live".to_string(),
            State::Online => "☁ online".to_string(),
            State::Coming(part) => format!("↓ {:.0}%", part * 100.0),
            State::Soft(_) => "⚠".to_string(),
        }
    }
}

/// How large a badge's `words` are set on a picture `width` wide:
/// [`BADGE`], or smaller when they are too long for a narrow picture,
/// such as a photo's pixels in the library, so they stay on it.
fn badge_size(words: &str, width: f32) -> f32 {
    let room = width - 2.0 * (BADGE_INSET + BADGE_PAD);
    BADGE.min(room / widgets::steady(words, 1.0))
}

/// The line under a step's or a result's name.
#[derive(Debug, PartialEq)]
pub(crate) enum Under {
    /// Where it is from and what it is: "Fixtures · picture".
    What(String),
    /// What is wrong with it, and whether that may sort itself out.
    Wrong(Level, String),
    /// It shows, but scaled up past its own pixels on a screen of this
    /// size.
    Soft(Pixels),
}

/// What is known of what a step shows, for its row.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Known {
    /// How long it plays by itself, known or guessed.
    length: Option<Estimate>,
    /// How many pages or slides it has, once they are loaded.
    pages: Option<usize>,
    /// How far its download has come, while it is under way.
    download: Option<f32>,
    /// A picture's or a video's own size, once its thumbnail is made.
    pixels: Option<Pixels>,
    /// The screen it is shown on (see [`crate::outputs::stage_screen`]).
    screen: Pixels,
    /// Where it comes from when shown.
    reach: Reach,
}

impl Known {
    /// The screen what it shows looks soft on, where it does.
    pub(crate) fn soft_on(self) -> Option<Pixels> {
        self.pixels
            .filter(|&pixels| looks_soft(pixels, self.screen))
            .map(|_| self.screen)
    }
}

impl<'a> RowFacts<'a> {
    fn of(
        step: &'a Step,
        issue: Option<&Issue>,
        known: Known,
        at: Option<TimeOfDay>,
        goes_on: bool,
    ) -> RowFacts<'a> {
        let thumb = match &step.action {
            Action::Show { resource } => Thumb::of(resource, known),
            Action::Live { .. } => Thumb::kind(Icon::Screen),
            Action::Blank => Thumb::kind(Icon::ScreenOff),
            Action::Note | Action::Section { .. } => Thumb::kind(Icon::Note),
        };
        let issue = issue.map(|issue| (issue.level, issue.what.clone()));
        let (thumb, under) = one_warning(thumb, issue, what_it_is(step));
        RowFacts {
            at,
            thumb,
            under,
            goes_on,
        }
    }
}

/// A row's picture and the line under its name, once `issue`, what would
/// go wrong showing it, has had its say: the line is `what` it is, or
/// the one warning that matters.
fn one_warning<'a>(
    thumb: Thumb<'a>,
    issue: Option<(Level, String)>,
    what: String,
) -> (Thumb<'a>, Under) {
    // A download under way says how far it has come on the picture:
    // that it isn't done yet is no news under the name. Something wrong
    // is the one warning a row gives, over a soft picture's.
    let (state, under) = match (thumb.state, issue) {
        (Some(State::Coming(_)), _) => (thumb.state, Under::What(what)),
        (_, Some((level, words))) => (None, Under::Wrong(level, words)),
        (Some(State::Soft(screen)), None) => (thumb.state, Under::Soft(screen)),
        (_, None) => (thumb.state, Under::What(what)),
    };
    (Thumb { state, ..thumb }, under)
}

/// What a search result's row says of it, on the computer and the phone
/// alike (see [`App::result_facts`]).
#[derive(Debug, PartialEq)]
pub(crate) struct ResultFacts<'a> {
    pub(crate) thumb: Thumb<'a>,
    pub(crate) under: Under,
}

impl<'a> ResultFacts<'a> {
    /// `resource`'s row, `known` what is known of it, `issue` what would
    /// go wrong showing it and `on_stage` whether it is on stage now.
    pub(crate) fn of(
        resource: &'a Resource,
        known: Known,
        issue: Option<(Level, String)>,
        on_stage: bool,
    ) -> ResultFacts<'a> {
        let (thumb, under) = one_warning(Thumb::of(resource, known), issue, what_of(resource));
        let state = match on_stage {
            true => Some(State::Live),
            false => thumb.state,
        };
        ResultFacts {
            thumb: Thumb { state, ..thumb },
            under,
        }
    }
}

/// How big `resource` is, in words: how long it plays, how many pages it
/// has, or a picture's pixels.
fn size_words(resource: &Resource, known: Known) -> Option<String> {
    match plays(resource) {
        true => known
            .length
            .map(length_words)
            .filter(|words| !words.is_empty()),
        false if matches!(resource.kind, Kind::Image) => {
            known.pixels.map(|pixels| pixels.to_string())
        }
        false if let Some(verses) = verses(resource) => Some(count(verses, "verse")),
        // A page opened out of its file is one page.
        false if resource.page_of().is_none() => {
            known.pages.map(|pages| count(pages, unit(resource)))
        }
        false => None,
    }
}

/// How many verses a Bible passage has.
fn verses(resource: &Resource) -> Option<usize> {
    let verses = resource.meta.get("verses")?;
    Some(verses.split(',').filter(|verse| !verse.is_empty()).count()).filter(|&n| n > 0)
}

/// What a line of the facts under the preview says.
#[derive(Debug, PartialEq)]
pub(crate) enum Says {
    Words(String),
    /// A section's name, to be in its colour, and words after it.
    Section(StepId, String, String),
}

/// The facts under the preview, line by line, each with its label. Only
/// what is known is said.
pub(crate) type Facts = Vec<(&'static str, Says)>;

/// The facts of the plan's step at `at`, `length` how long it is in
/// words and `known` what is known of what it shows: where it is from, a
/// picture's size, how it plays and goes on, whose look it has and how it
/// fades, and where in the plan it is.
pub(crate) fn step_facts(plan: &Plan, at: usize, length: &str, known: Known) -> Facts {
    let step = &plan.steps()[at];
    let mut facts = Facts::new();
    let from = match &step.action {
        Action::Show { resource } => from_words(resource),
        Action::Live {
            hint: Some(query), ..
        } => Some(query.text.trim().to_string()).filter(|text| !text.is_empty()),
        _ => None,
    };
    facts.extend(from.map(|from| ("From", Says::Words(from))));
    if let Action::Show { resource } = &step.action {
        facts.extend(picture_size(resource, known).map(|size| ("Size", Says::Words(size))));
    }
    let goes_on = match plan.goes_on(at) {
        true => "goes on by itself",
        false => "waits",
    };
    facts.push((
        "Plays",
        Says::Words(match length.is_empty() {
            true => goes_on.to_string(),
            false => format!("{length}, then {goes_on}"),
        }),
    ));
    if matches!(step.action, Action::Show { .. } | Action::Live { .. }) {
        let own = |step: &Step| step.look.as_ref().is_some_and(|look| !look.is_empty());
        let whose = match own(step) {
            true => "its own".to_string(),
            false => plan
                .within(at)
                .into_iter()
                .rev()
                .filter_map(|id| plan.get(id))
                .find(|section| own(section))
                .map_or("the plan's".to_string(), |section| {
                    format!("{}'s", section.label)
                }),
        };
        let look = std::iter::once(whose)
            .chain(fade_words(step.fade))
            .collect::<Vec<_>>()
            .join(" · ");
        facts.push(("Look", Says::Words(look)));
    }
    if let Some(section) = plan.within(at).last().and_then(|id| plan.get(*id)) {
        let steps = |upto: usize| {
            plan.steps()[..upto]
                .iter()
                .filter(|step| !step.is_section())
                .count()
        };
        let which = format!("step {} of {}", steps(at + 1), steps(plan.steps().len()));
        facts.push((
            "Section",
            Says::Section(section.id, section.label.clone(), which),
        ));
    }
    facts
}

/// The facts of `resource`, which isn't in the plan, `frames` how many
/// slides or pages it is known to have: where it is from, what it is,
/// and how long or how big it is.
pub(crate) fn item_facts(resource: &Resource, known: Known, frames: Option<usize>) -> Facts {
    let mut facts = Facts::new();
    facts.extend(from_words(resource).map(|from| ("From", Says::Words(from))));
    facts.extend(kind_word(resource).map(|what| ("What", Says::Words(what.to_string()))));
    let known = Known {
        pages: known.pages.or(frames),
        ..known
    };
    let size = match plays(resource) {
        true => "Length",
        false => "Size",
    };
    let words = picture_size(resource, known).or_else(|| size_words(resource, known));
    facts.extend(words.map(|words| (size, Says::Words(words))));
    facts
}

/// A picture's size as the preview's facts say it: "1920 × 1080".
fn picture_size(resource: &Resource, known: Known) -> Option<String> {
    let pixels = known
        .pixels
        .filter(|_| matches!(resource.kind, Kind::Image))?;
    Some(format!("{} × {}", pixels.width, pixels.height))
}

/// Where `resource` is from: its folder and its file's name ("Fixtures /
/// video.mp4"), or its source and what it is called there ("RV1909 ·
/// Juan 3:16").
fn from_words(resource: &Resource) -> Option<String> {
    let (name, between) = match &resource.location {
        Location::File(path) => (path.file_name()?.to_string_lossy().into_owned(), " / "),
        _ => (resource.title.clone(), " · "),
    };
    Some(
        resource
            .subtitle
            .iter()
            .cloned()
            .chain([name])
            .filter(|words| !words.is_empty())
            .collect::<Vec<_>>()
            .join(between),
    )
    .filter(|from| !from.is_empty())
}

/// How a step fades, in words: "fades in 2 s, out 4 s"; none when it
/// doesn't.
fn fade_words(fade: lantern_engine::Fade) -> Option<String> {
    match (fade.fade_in, fade.fade_out) {
        (None, None) => None,
        (Some(fade_in), None) => Some(format!("fades in {} s", fade_in.secs())),
        (None, Some(fade_out)) => Some(format!("fades out {} s", fade_out.secs())),
        (Some(fade_in), Some(fade_out)) => Some(format!(
            "fades in {} s, out {} s",
            fade_in.secs(),
            fade_out.secs()
        )),
    }
}

/// Whether `resource` plays for a time of its own: a video, a sound, a
/// countdown.
fn plays(resource: &Resource) -> bool {
    matches!(resource.kind, Kind::Video | Kind::Audio)
        || resource.meta.contains_key(Resource::DURATION)
}

/// Where what a step shows is from and what it is, "Fixtures · picture";
/// for a step that shows nothing of its own, what it does instead.
fn what_it_is(step: &Step) -> String {
    let resource = match &step.action {
        Action::Show { resource } => resource,
        Action::Live { hint, shown } => return slot_detail(hint.as_ref(), shown.len()),
        Action::Blank => return "nothing on the projector".into(),
        Action::Note => return "a note for you, nothing on stage".into(),
        Action::Section { .. } => return String::new(),
    };
    what_of(resource)
}

/// Where `resource` is from and what it is: "Fixtures · picture".
pub(crate) fn what_of(resource: &Resource) -> String {
    resource
        .subtitle
        .iter()
        .map(String::as_str)
        .chain(kind_word(resource))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// What `resource` is, in a word: "picture", "video", "page".
pub(crate) fn kind_word(resource: &Resource) -> Option<&str> {
    match (&resource.kind, resource.page_of()) {
        (_, Some(_)) => Some(unit(resource)),
        (Kind::Image, _) => Some("picture"),
        (Kind::Video, _) => Some("video"),
        (Kind::Audio, _) => Some("sound"),
        (Kind::Document, _) => Some("document"),
        (Kind::Presentation, _) => Some("slides"),
        (Kind::Web, _) => Some("web page"),
        // A text's source says what it is: "your text", a Bible's name.
        (Kind::Text, _) => resource.subtitle.is_none().then_some("text"),
        (Kind::Other(name), _) => Some(name.as_str()),
    }
}

/// A length as a row says it: `4:00` when known, `~4 min` when guessed,
/// nothing for a guess of nothing.
fn length_words(estimate: Estimate) -> String {
    match estimate.basis {
        Basis::Guess if estimate.length.is_zero() => String::new(),
        Basis::Guess => length::rough(estimate.length),
        Basis::Set | Basis::Known => length::clock(estimate.length),
    }
}

impl App {
    /// The plan's pane, `width` wide.
    pub(crate) fn plan_pane(&self, width: f32, timing: &Timing) -> Element<'_, Message> {
        let p = self.palette;
        let plan = self.engine.0.plan();
        let issues = self.issues(&plan);
        // What comes and goes (the notice, readiness) sits under the steps,
        // in one child. Above them it pushed every row down as it came and
        // up as it went, under the pointer. And the steps must stay the
        // same child of the pane: iced keeps a widget's state, like which
        // field has focus, by its place among its siblings.
        // The plan's name and its buttons are in the strip, over the pane.
        let mut pane = column![self.mode_note()].spacing(8);
        pane = pane.push(match &self.plans.mode {
            plans::Mode::Steps | plans::Mode::Renaming(_) => self.steps(&plan, timing, &issues),
            plans::Mode::Browsing(entries) => self.saved_plans(entries),
            plans::Mode::Confirm(switch) => confirm(p, &plan, switch, false),
            plans::Mode::Leaving(switch) => confirm(p, &plan, switch, true),
        });
        let mut status = row![].spacing(8).align_y(Alignment::Center);
        // Readiness only when something isn't: "ready" would be noise here.
        if !issues.is_empty()
            && let Some(readiness) = self.readiness(&plan, &issues)
        {
            status = status.push(readiness);
        }
        if let Some(notice) = self.plan_notice(&plan) {
            status = status.push(notice);
        }
        // While planning, a section carried says at the foot how to set
        // how far in it goes, past what the row under the pointer says.
        if self.mode == crate::mode::Mode::Plan
            && self.carrying_section()
            && matches!(
                self.plans.mode,
                plans::Mode::Steps | plans::Mode::Renaming(_)
            )
        {
            let hint = row![
                text("← →").size(META).font(BOLD).color(p.primary),
                text("a level out or in")
                    .size(META)
                    .color(p.on_surface_variant),
            ]
            .spacing(6)
            .align_y(Alignment::Center);
            pane = pane.push(container(hint).padding([0, 8]));
        }
        pane = pane.push(container(status).padding([0, 8]));
        // While planning the plan is the page, with a page's margins: at
        // its sides about a nineteenth of its width, 36 at a laptop's,
        // never less than in a run.
        let padding = match self.mode {
            crate::mode::Mode::Plan => {
                let side = (width * 0.053).round().clamp(12.0, 36.0);
                iced::Padding::from([20.0, side]).bottom(16.0)
            }
            _ => iced::Padding::from([16, 12]),
        };
        let pane = pane.padding(padding).height(Fill);
        iced::widget::stack![pane, self.refusal_over()]
            .width(Fill)
            .height(Fill)
            .into()
    }

    /// The plan's name and what is beside it in the strip, `room` wide.
    /// While `planning` the name drops the list of plans and the sliders
    /// beside it open the plan's own settings; its facts are in its title
    /// over its rows. In a run it is only a name, with where the service
    /// is: "3 of 7". Then Save, while the plan has changes.
    pub(crate) fn plan_header(
        &self,
        plan: &Plan,
        planning: bool,
        room: f32,
    ) -> Element<'_, Message> {
        let p = self.palette;
        // A long name is cut at its edge, not drawn over what follows. It
        // is renamed in the plan's settings.
        let name = container(
            single_line(plan.title.clone())
                .size(HEADING)
                .font(BOLD)
                .color(p.primary),
        )
        .clip(true);
        let items: Vec<&Step> = plan.steps().iter().filter(|s| !s.is_section()).collect();
        let save: Option<Element<'_, Message>> = if self.plans.is_dirty(plan) {
            Some(hinted(
                p,
                pill(p, "Save", Tone::Selected, PILL_SMALL, Some(op(Op::Save))),
                keys::with_key("Save the plan", Op::Save),
                tooltip::Position::Bottom,
            ))
        } else {
            // Saved: Save is a tick for a moment, fading.
            self.motion.tick().map(|shown| {
                container(look::icon(
                    Icon::Check,
                    14.0,
                    iced::Color {
                        a: shown,
                        ..p.primary
                    },
                ))
                .padding([3, 10])
                .into()
            })
        };
        let mut header = row![].spacing(6).align_y(Alignment::Center);
        if planning {
            let browsing = matches!(self.plans.mode, plans::Mode::Browsing(_));
            // The sliders' button, and Save or its tick: the name keeps
            // them their room.
            const TUNE: f32 = 35.0;
            let saving = match (self.plans.is_dirty(plan), save.is_some()) {
                (true, _) => widgets::steady("Save", BODY) + 32.0,
                (false, true) => 34.0,
                (false, false) => 0.0,
            };
            // The pill round the name: its padding and the chevron.
            const AROUND: f32 = 40.0;
            let name_room = room - TUNE - saving - 18.0 - AROUND;
            // Pressed, the list of plans takes the plan's place, the name
            // lit while it shows; pressed again, the plan is back.
            let chevron = look::icon(Icon::Down, 14.0, p.primary).rotation(match browsing {
                true => iced::Radians::PI,
                false => iced::Radians(0.0),
            });
            header = header.push(hinted(
                p,
                button(
                    row![name.max_width(name_room.max(0.0)), chevron]
                        .spacing(6)
                        .align_y(Alignment::Center),
                )
                .padding([4, 10])
                .on_press(op(Op::Open))
                .style(look::pill(
                    p,
                    if browsing {
                        Tone::Selected
                    } else {
                        Tone::Plain
                    },
                )),
                keys::with_key("Your plans", Op::Open),
                tooltip::Position::Bottom,
            ));
            let open = self.plan_panel;
            header = header.push(
                tooltip(
                    button(look::icon(
                        Icon::Tune,
                        15.0,
                        if open {
                            p.primary
                        } else {
                            p.on_surface_variant
                        },
                    ))
                    .padding([9, 10])
                    .on_press(Message::PlanSettings)
                    .style(look::strip_item(
                        p,
                        if open { Tone::Selected } else { Tone::Plain },
                    )),
                    container(
                        text("This plan: its name, times, media, look, copy, delete").size(META),
                    )
                    .padding([4, 8])
                    .style(look::tip(p)),
                    tooltip::Position::Bottom,
                )
                .delay(look::TIP_DELAY),
            );
        } else {
            let open = self.plan_panel;
            header = header.push(
                tooltip(
                    button(row![name].spacing(8).align_y(Alignment::Center))
                        .padding([4, 10])
                        .style(look::pill(
                            p,
                            if open { Tone::Selected } else { Tone::Plain },
                        )),
                    container(text("This plan: media, look, copy, delete").size(META))
                        .padding([4, 8])
                        .style(look::tip(p)),
                    tooltip::Position::Bottom,
                )
                .delay(look::TIP_DELAY),
            );
            // Where the service is shows only once it's running.
            let at = self
                .engine
                .0
                .current_step()
                .and_then(|current| items.iter().position(|s| s.id == current.id));
            if let Some(at) = at {
                header = header.push(
                    text(format!("{} of {}", at + 1, items.len()))
                        .size(META)
                        .color(p.on_surface_variant),
                );
            }
        }
        header.push(save).into()
    }

    /// The last save or open: its error, or that it worked while nothing
    /// has changed since.
    fn plan_notice(&self, plan: &Plan) -> Option<Element<'_, Message>> {
        let p = self.palette;
        let (message, color) = match self.plans.notice.as_ref()? {
            Notice::Failed(why) => (why, p.error),
            Notice::Done(_) if self.plans.is_dirty(plan) => return None,
            Notice::Done(what) => (what, p.on_surface_variant),
        };
        // On as many lines as it takes: an export's result or an error's
        // reason is the part that was cut off.
        Some(
            text(message.as_str())
                .size(META)
                .color(color)
                .width(Fill)
                .into(),
        )
    }

    fn steps(&self, plan: &Plan, timing: &Timing, issues: &[Issue]) -> Element<'_, Message> {
        let p = self.palette;
        let step_rows = Rows::of(self.mode);
        let current = self.engine.0.current_step().map(|step| step.id);
        let next = self.engine.0.up_next().map(|step| step.id);
        let at = current.and_then(|id| plan.position(id));
        // The last of the steps picked together has what can be done to
        // them at its end.
        let picked_end = self
            .picked
            .iter()
            .filter_map(|id| plan.position(*id))
            .max()
            .map(|last| (last..plan.group_end(last)).last().unwrap_or(last))
            .filter(|_| {
                self.picked.len() > 1
                    && self.can_edit()
                    && self.drag.as_ref().is_none_or(|drag| !drag.moving)
            })
            .map(|last| plan.steps()[last].id);
        // What a shuffle has given already this round, wherever it stands.
        let drawn = self.engine.0.drawn_before();
        // While a drag moves, its rows leave the list and a gap opens where
        // they'd land; the editing strip and marks step aside.
        let moving = self.drag.as_ref().filter(|drag| drag.moving);
        let carried = moving.map_or(&[][..], |drag| drag.carried.as_slice());
        // What is carried in from the library opens the same gap.
        let held = self.carried.as_ref().filter(|held| held.moving);
        // Where it would land, as the plan has it: before which of the
        // rows that stay, how far in, and in which section.
        let target = self.carry_target(plan);
        let landing = target.map(|(at, ..)| at);
        // It opens as it comes, so the rows under it are moved aside and
        // not thrown there.
        let opening = self.motion.gap();
        // The gap it just left closes at the same pace, so the rows in
        // between slide over instead of jumping.
        let closing = self.motion.closing_gap();
        let gap_level = target.map_or(0, |(_, level, _)| level);
        // The gap is that far in, sliding there when the level changes.
        let gap_in = self
            .motion
            .gap_indent()
            .unwrap_or(INDENT * f32::from(gap_level));
        // A section says which levels it could take there, and which it
        // has: one mark a level under its gap.
        let ruler = moving
            .filter(|drag| drag.is_section())
            .map(|drag| (drag.level, drag.levels));
        let gap_tall = GAP_TALL + ruler.map_or(0.0, |_| RULER_TALL);
        // The place that opens is in the shape of what is carried. For
        // steps, a dashed place for each (up to a few), with a dot's
        // place on the line, so a loop or a drawn order they bring is
        // drawn beside them. For a section, a dashed header at its level.
        // A file from the library has a step's place.
        let stepwise: Vec<StepId> = moving
            .filter(|drag| !drag.is_section())
            .map(|drag| drag.carried.iter().copied().take(PLACES).collect())
            // Files carried in from the library have a step's place each.
            .or_else(|| {
                held.map(|held| {
                    (0..(1 + held.more.len()).min(PLACES))
                        .map(held_place)
                        .collect()
                })
            })
            .unwrap_or_default();
        let blockwise = moving
            .filter(|drag| drag.is_section())
            .map(|drag| drag.step);
        let gap_tall = match (stepwise.len(), blockwise) {
            (0, None) => gap_tall,
            (0, Some(_)) => step_rows.head_room() + HEAD_LINE + ruler.map_or(0.0, |_| RULER_TALL),
            (places, _) => step_rows.place() * places as f32,
        };
        let closing_gap = move |left: f32| Space::new().height(gap_tall * left);
        // The sections what is carried would be in, innermost first.
        let into: Vec<StepId> = target
            .and_then(|(.., section)| section)
            .map(|section| {
                let around = plan
                    .position(section)
                    .map_or_else(Vec::new, |at| plan.within(at));
                std::iter::once(section)
                    .chain(around.into_iter().rev())
                    .collect()
            })
            .unwrap_or_default();
        let colors = plan.section_colors();
        let hue = |id: StepId| section_hue(p, &colors, id);
        // The places that open are in the colour of the section they
        // would land in.
        let gap_ink = into.first().map_or(p.primary, |section| hue(*section));
        let gap = |lanes: Lanes<'_>| -> Element<'_, Message> {
            let dashes = |tall: f32| {
                crate::drawn::dashed(
                    container(Space::new().width(Fill).height(tall)).style(move |_| {
                        container::Style {
                            background: Some(Palette::tint(gap_ink, 0.08).into()),
                            border: iced::Border {
                                radius: 9.0.into(),
                                ..iced::Border::default()
                            },
                            ..container::Style::default()
                        }
                    }),
                    Palette::tint(gap_ink, 0.7),
                    9.0,
                )
            };
            // The same room a step's rail takes, with where its dot would
            // be.
            let beside = |id: StepId, dot: Option<f32>| -> Element<'_, Message> {
                let mark = match dot {
                    Some(down) => column![
                        Space::new().height(down),
                        track::seen(
                            track::Key::Dot(id),
                            &self.places,
                            Space::new().width(RAIL_DOT).height(RAIL_DOT)
                        ),
                    ],
                    None => column![],
                };
                row![
                    Space::new().width(lanes.left()),
                    track::seen(
                        track::Key::Row(id),
                        &self.places,
                        mark.width(RAIL).height(Fill).align_x(Alignment::Center)
                    ),
                    Space::new().width(lanes.drawn)
                ]
                .height(Fill)
                .into()
            };
            let slot: Element<'_, Message> = match (stepwise.is_empty(), blockwise) {
                (false, _) => column(stepwise.iter().map(|id| {
                    row![
                        beside(*id, Some(lanes.rows.place_dot())),
                        container(dashes(lanes.rows.place() - 8.0)).padding([4, 10]),
                    ]
                    .height(lanes.rows.place())
                    .into()
                }))
                .into(),
                // A section: one dashed header at its level, its name
                // faint, with the levels it could take under it.
                (true, Some(section)) => {
                    let (room, tall) = (lanes.rows.head_room(), HEAD_LINE);
                    let name = plan.get(section).map(|step| step.label.clone());
                    let head = crate::drawn::dashed(
                        container(
                            single_line(name.unwrap_or_default())
                                .size(NAME)
                                .font(BOLD)
                                .color(Palette::tint(gap_ink, 0.7)),
                        )
                        .padding([0, 10])
                        .width(Fill)
                        .center_y(tall),
                        Palette::tint(gap_ink, 0.7),
                        9.0,
                    );
                    let mut block = column![
                        row![
                            beside(section, None),
                            container(head).padding(
                                iced::Padding::default()
                                    .top(room)
                                    .left(10.0 + gap_in)
                                    .right(10.0)
                            ),
                        ]
                        .height(room + tall)
                    ];
                    if let Some((at, (least, most))) = ruler {
                        block =
                            block.push(container(level_ruler(p, at, least, most)).padding(
                                iced::Padding::default().left(lanes.left() + RAIL + gap_in),
                            ));
                    }
                    block.into()
                }
                (true, None) => container(drop_gap(
                    p,
                    held.map(crate::shelf::Carried::title).unwrap_or_default(),
                ))
                .padding(iced::Padding::default().left(gap_in))
                .into(),
            };
            match opening {
                Some(open) => container(slot)
                    .max_height(gap_tall * open)
                    .clip(true)
                    .into(),
                None => slot,
            }
        };
        // Where the gap's first and last places say they are: for steps,
        // their dots; for a section, its line.
        let gap_keys = match (stepwise.first(), stepwise.last(), blockwise) {
            (Some(first), Some(last), _) => Some((track::Key::Dot(*first), track::Key::Dot(*last))),
            (_, _, Some(section)) => Some((track::Key::Row(section), track::Key::Row(section))),
            _ => None,
        };
        // The pink line is there only while a search has a result for
        // Ctrl+Enter to add.
        let adds = self.search.first().is_some();
        let mut rows = column![].spacing(2);
        // What was just refused shakes and flashes, the wrong colour and
        // white by turns, while the refusal is said: how far aside it is.
        let alarm = Some(self.motion.nudge()).filter(|aside| *aside != 0.0);
        let alarm_ink = match alarm {
            Some(aside) if aside > 0.0 => iced::Color::WHITE,
            _ => p.error,
        };
        // Where the dots are, said by the rows as they are laid out, and
        // what one drawing over the list makes of them.
        let places = self.places.clone();
        // Folded by hand, or over in a service: its header stands for all
        // it holds.
        let shut = |id: StepId| self.shut(timing, id);
        // A section's foot being pulled where it can't end says so in the
        // colour of what is wrong.
        let foot_refused = match self.foot {
            Some((section, Some(last))) => plan
                .clone()
                .reach_section(section, Some(last))
                .is_err()
                .then_some(section),
            _ => None,
        };
        let mut stops: Vec<track::Stop> = Vec::new();
        // The place for what is carried is in sight.
        let mut gapped = false;
        // Where every row in sight says it is: a press on one of them is
        // not on nothing.
        let mut seen_rows: Vec<track::Key> = Vec::new();
        // The folded sections in sight: each one row for all it holds.
        let mut gathered: Vec<usize> = Vec::new();
        let mut shown: Vec<usize> = Vec::new();
        // While planning, the pane is a tree: the plan's sections and
        // their steps.
        let tree = self.mode == crate::mode::Mode::Plan;
        let running = self.mode == crate::mode::Mode::Run;
        let can = self.can_edit();
        // Its head is over its rows while planning, and in a service where
        // it can change.
        let headed = matches!(self.mode, crate::mode::Mode::Plan | crate::mode::Mode::Run)
            && can
            && !plan.is_empty();
        // While planning, where the plan can change, a section's head and
        // foot are pulled to start or end it somewhere else.
        let feet = tree && can;
        let loops = laid(plan, |flow| flow.repeat != Repeat::Once);
        let orders = laid(plan, |flow| flow.shuffle);
        // While planning one of each is always there: a pull can draw
        // either line, and the rows must not shift as it begins.
        let least = u8::from(tree && can);
        let deepest = |of: &[Laid]| of.iter().map(|laid| laid.lane + 1).max().unwrap_or(0);
        let (loop_lanes, order_lanes) = (deepest(&loops).max(least), deepest(&orders).max(least));
        let lanes = Lanes {
            // A plan with no start has no times of day to say.
            time: match (step_rows, plan.starts) {
                (Rows::Tall, Some(_)) => TIME_WIDE,
                _ => 0.0,
            },
            curl: track::loop_room(loop_lanes) - RAIL / 2.0,
            drawn: track::order_room(order_lanes) - RAIL / 2.0,
            loops: &loops,
            orders: &orders,
            loop_lanes,
            order_lanes,
            rows: step_rows,
        };
        if running {
            rows = rows.push(self.run_time_row(plan, timing));
        }
        if headed {
            rows = rows.push(self.plan_head(plan, timing));
        }
        // A service locked is watched, not changed: its rail is the road
        // it takes.
        if running && !can {
            rows = rows.push(self.road_rail(plan, timing, issues));
            return mouse_area(
                scrolling(rows)
                    .id(ROWS)
                    .on_scroll(|seen| Message::PlanScrolled(seen.absolute_offset().y)),
            )
            .on_exit(Message::LeftPlan)
            .into();
        }
        // The time of day each step first comes, where the plan has a
        // start.
        let clock = plan.starts.map(|starts| {
            let schedule = lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step));
            move |id| schedule.first_at(id).map(|after| starts.plus(after))
        });
        // Where a fill's proposed rows go: the end of its section, as far
        // in as that section's steps.
        let fill_of = self
            .filling
            .as_ref()
            .and_then(|fill| plan.position(fill.section));
        let fill_at = fill_of.map(|at| plan.group_end(at));
        let fill_indent = fill_of.map_or(0.0, |at| {
            INDENT * f32::from(plan.level(at) + 1) + lanes.left()
        });
        // The steps and what sits among them, with no room between two:
        // each brings its own under it, but for a step's rail, which runs
        // the whole of its row so the line down the dots is unbroken.
        let mut list = column![];
        macro_rules! put {
            ($row:expr $(,)?) => {
                list = list.push(apart($row))
            };
        }
        // A step in a folded section stays in sight while the service is
        // at it or it is being worked on.
        let kept: Vec<StepId> = current
            .into_iter()
            .chain(self.selected.filter(|_| moving.is_none()))
            .collect();
        let rows_listed = listed(
            plan,
            &shut,
            &kept,
            carried,
            landing.map(|at| (at, gap_level)),
            feet,
        );
        // Where a row is in the plan, and among the rows that stay: the
        // gap that is closing goes before the first row at or after its
        // own. What a fill proposes goes at the end of its section, before
        // its foot, or else before what follows.
        let index_of = |row: &Listed| match *row {
            Listed::Step(id) | Listed::Head { section: id, .. } => plan.position(id),
            _ => None,
        };
        let rest_at = |index: usize| {
            index
                - carried
                    .iter()
                    .filter(|id| plan.position(**id).is_some_and(|at| at < index))
                    .count()
        };
        let fill_section = self.filling.as_ref().map(|fill| fill.section);
        let mut closed = closing.is_none();
        let mut filled = fill_at.is_none();
        for row in &rows_listed {
            let index = index_of(row);
            if let (Some(index), Some((at, left))) = (index, closing.filter(|_| !closed))
                && rest_at(index) >= at
            {
                put!(closing_gap(left));
                closed = true;
            }
            let fills = match *row {
                Listed::Foot(section) => fill_section == Some(section),
                _ => index.zip(fill_at).is_some_and(|(index, at)| index >= at),
            };
            if fills && !filled {
                filled = true;
                if let Some(proposed) = self.fill_rows(fill_indent) {
                    put!(track::seen(track::Key::Fill, &places, proposed));
                }
            }
            let index = match *row {
                Listed::Gap => {
                    put!(gap(lanes));
                    gapped = true;
                    continue;
                }
                Listed::Foot(section) => {
                    list = list.push(foot_strip(section, &places));
                    continue;
                }
                Listed::Empty(section) => {
                    seen_rows.push(track::Key::Place(section));
                    put!(self.empty_place(
                        section,
                        hue(section),
                        picked_end == Some(section),
                        lanes,
                        &places
                    ));
                    continue;
                }
                Listed::Head { .. } | Listed::Step(_) => match index {
                    Some(index) => index,
                    None => continue,
                },
            };
            let step = &plan.steps()[index];
            let selected = self.selected == Some(step.id) && moving.is_none();
            // Rows just taken away from before this one close up.
            if let Some(room) = self.motion.gone_before(Some(step.id)) {
                put!(Space::new().height(room));
            }
            if let Listed::Head { depth, folded, .. } = *row {
                seen_rows.push(track::Key::Whole(step.id));
                put!(self.header(
                    plan,
                    timing,
                    index,
                    depth,
                    folded,
                    hue(step.id),
                    lanes,
                    &places
                ));
                // Folded, its steps are its stacked dot, and their line is
                // its last step's: down to what follows, where that goes on
                // into it by itself.
                if folded {
                    gathered.push(index);
                    let over = self.over(timing, step.id);
                    let last = (index + 1..plan.group_end(index))
                        .rev()
                        .find(|at| !plan.steps()[*at].is_section());
                    let after = last.and_then(|last| plan.next_step(last));
                    let goes = last.is_some_and(|last| plan.goes_on(last)) && after.is_some();
                    let ink = match over {
                        true => p.outline,
                        false => hue(step.id),
                    };
                    stops.push(track::Stop {
                        id: step.id,
                        dot: track::Dot::Stack {
                            color: match over {
                                true => Palette::tint(hue(step.id), 0.45),
                                false => hue(step.id),
                            },
                            solid: over,
                        },
                        line: goes.then_some(track::Line::Plain(ink)),
                        next: after.map(|after| plan.steps()[after].id),
                        back: None,
                    });
                }
                if selected && adds {
                    put!(insertion_mark(p, INDENT));
                }
                continue;
            }
            let place = if self
                .rehearsal
                .as_ref()
                .is_some_and(|rehearsal| rehearsal.seen(step.id))
            {
                // What a rehearsal has been through is past, for now.
                Place::Done
            } else if Some(step.id) == current {
                Place::Current
            } else if Some(step.id) == next {
                Place::Next
            } else if at.is_some_and(|at| index < at) || drawn.contains(&step.id) {
                // Above the step that is on, or drawn before it in a
                // shuffle.
                Place::Done
            } else {
                Place::Later
            };
            let issue = issues.iter().find(|i| i.step == step.id);
            let noted = crate::notes::numbers_on(plan, lantern_engine::NoteOn::Step(step.id));
            // While planning, its row says all of what it is.
            let facts = (step_rows == Rows::Tall).then(|| {
                RowFacts::of(
                    step,
                    issue,
                    self.known(step),
                    clock.as_ref().and_then(|clock| clock(step.id)),
                    plan.goes_on(index),
                )
            });
            let mut row = column![self.ringed(
                crate::rail::RingOn::Dot(step.id),
                self.draggable(
                    step.id,
                    self.step_row(
                        step,
                        place,
                        issue,
                        facts.as_ref(),
                        &noted,
                        picked_end == Some(step.id)
                    )
                )
            )];
            // In a service, what is noted of the step that is on is said
            // under it: its own notes, its loops', and its sections' with
            // the first of their steps to come.
            if place == Place::Current && running {
                for (_, n, words) in
                    crate::notes::on_step(plan, step.id, true, &|id| self.engine.0.reached(id))
                {
                    row = row.push(
                        container(crate::notes::said(p, n, words))
                            .padding(iced::Padding::default().top(4.0)),
                    );
                }
            }
            // The result would go after the selected step, or with nothing
            // selected after the current one.
            let after = selected
                || (self.selected.is_none() && Some(step.id) == current && moving.is_none());
            if after && adds {
                row = row.push(insertion_mark(p, 0.0));
            }
            // Inside the step: what its slides play or link to, and the
            // sound that plays with it.
            let parts = self.step_parts(step, place == Place::Current, step_rows.inset());
            let tall = ROW_TALL + parts.len() as f32 * PART_TALL;
            for part in parts {
                row = row.push(part);
            }
            // The rail runs down beside the step and what is inside it.
            // The rail is as tall as the row and the room under it, so
            // its line meets the next row's.
            let tint = plan
                .within(index)
                .last()
                .map_or(p.primary, |section| hue(*section));
            let (rail, mut stop) = self.rail(plan, index, place, tint, lanes, &places);
            // A step that was not let go where it was carried shakes.
            let aside = alarm
                .filter(|_| self.alarm.0.contains(&step.id))
                .map_or(0.0, f32::abs);
            if aside > 0.0 {
                stop.dot = track::Dot::Would(alarm_ink);
            }
            // Left of the rail, the time of day it comes, level with its
            // dot.
            let time: Element<'_, Message> = match facts.as_ref().and_then(|facts| facts.at) {
                Some(at) => container(text(at.to_string()).size(META).color(p.outline))
                    .padding(iced::Padding::default().right(8.0))
                    .align_right(lanes.time)
                    .center_y(TALL_LINE)
                    .into(),
                None => Space::new().width(lanes.time).into(),
            };
            let row =
                row![Space::new().width(aside), time, rail, apart(row)].height(Length::Shrink);
            seen_rows.push(track::Key::Whole(step.id));
            let row = track::seen(track::Key::Whole(step.id), &places, row);
            // Put back just now: it grows in.
            let row: Element<'_, Message> = match self.motion.growing(step.id) {
                Some(grown) => container(row).max_height(tall * grown).clip(true).into(),
                None => row,
            };
            // Put in by a fill just now: its proposal's dashed outline fades.
            let row: Element<'_, Message> = match self.motion.solidifying(step.id) {
                Some(t) => crate::drawn::dashed(
                    container(row),
                    iced::Color {
                        a: 0.6 * (1.0 - t),
                        ..p.primary
                    },
                    12.0,
                ),
                None => row,
            };
            list = list.push(row);
            stops.push(stop);
            shown.push(index);
        }
        if let Some(room) = self.motion.gone_before(None) {
            put!(Space::new().height(room));
        }
        if !filled && let Some(proposed) = self.fill_rows(fill_indent) {
            put!(track::seen(track::Key::Fill, &places, proposed));
        }
        if let Some((_, left)) = closing.filter(|_| !closed) {
            put!(closing_gap(left));
        }
        rows = rows.push(list);
        if adds
            && self.selected.is_none()
            && current.is_none()
            && !plan.is_empty()
            && moving.is_none()
        {
            rows = rows.push(insertion_mark(p, 0.0));
        }
        if plan.is_empty() {
            rows = rows.push(
                container(
                    text(format!(
                        "The plan is empty. Search, then {} adds the result here.",
                        keys::label(Op::AddToPlan)
                    ))
                    .size(BODY)
                    .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        // The last row adds a section. Steps come from the library and
        // its search; Live and Black screen are first in the library.
        if moving.is_none() && self.can_edit() {
            // An empty plan's + breathes a few times: it is where to begin.
            let breath = match plan.is_empty() {
                true => self.motion.breath(),
                false => 0.0,
            };
            rows = rows.push(breathing_action(
                Palette {
                    on_surface_variant: crate::motion::blend(
                        p.on_surface_variant,
                        p.primary,
                        breath,
                    ),
                    ..p
                },
                "+",
                "Add a section".to_string(),
                Message::AddStep(NewStep::Section),
                breath,
            ));
        }
        // Under the steps, what's kept ready outside their order.
        if moving.is_none() && !plan.standby.is_empty() {
            rows = rows.push(Space::new().height(10));
            rows = rows.push(
                container(text("Standby").size(BODY).font(BOLD).color(p.primary)).padding([4, 10]),
            );
            let lined_up = self.standby_lined_up();
            for (at, resource) in plan.standby.iter().enumerate() {
                rows = rows.push(self.standby_row(resource, lined_up == Some(at)));
            }
        }
        // The spans beside the line: each from the first to the last of
        // its steps in sight.
        let id_at = |at: usize| plan.steps()[at].id;
        let span = |laid: &Laid, color, mark, pulled| {
            // A folded section's row stands for all it holds.
            let reach = |at: usize| match gathered.contains(&at) {
                true => (at, plan.group_end(at) - 1),
                false => (at, at),
            };
            let mut inside: Vec<usize> = shown
                .iter()
                .chain(&gathered)
                .copied()
                .filter(|at| {
                    let (top, foot) = reach(*at);
                    top <= laid.last && laid.first <= foot
                })
                .collect();
            inside.sort_unstable();
            let (&from, &to) = (inside.first()?, inside.last()?);
            if from == to {
                return None;
            }
            Some(track::Span {
                first: id_at(from),
                last: id_at(to),
                starts: reach(from).0 <= laid.first,
                ends: reach(to).1 >= laid.last,
                lane: laid.lane,
                color,
                mark,
                pulled,
                shift: 0.0,
                pulse: self.motion.round_pulse(laid.of),
                marks: match laid.of {
                    FlowOf::Span(id) => {
                        crate::notes::numbers_on(plan, lantern_engine::NoteOn::Span(id))
                            .into_iter()
                            .map(|(_, n)| n)
                            .collect()
                    }
                    _ => Vec::new(),
                },
            })
        };
        // In a service, the span the flow goes back round when the step
        // that is on ends is the stage's colour, and one the service is
        // in says which time round it is.
        // What letting go of the steps being carried would do to the
        // loops and drawn orders they are in.
        let (would_cut, would_leave) = match self.carry_check(plan) {
            Some(Err(lantern_engine::CarryError::Crosses(spans))) => (spans, Vec::new()),
            Some(Ok(carried)) => (Vec::new(), carried.left),
            _ => (Vec::new(), Vec::new()),
        };
        // A join is the line between its dots: one that letting go would
        // cut, or that was just not cut, is coloured as a loop's would be.
        let refused = !would_cut.is_empty() || alarm.is_some();
        for stop in stops.iter_mut().filter(|_| refused) {
            let Some(track::Line::Plain(_)) = stop.line else {
                continue;
            };
            let joins = plan
                .position(stop.id)
                .map_or_else(Vec::new, |at| plan.joins_of(at));
            if alarm.is_some() && joins.iter().any(|id| self.alarm.1.contains(id)) {
                stop.line = Some(track::Line::Plain(alarm_ink));
            } else if joins.iter().any(|id| would_cut.contains(id)) {
                stop.line = Some(track::Line::Plain(p.error));
            }
        }
        let at_now = at.filter(|_| running);
        let back = at_now.and_then(|_| self.engine.0.follows_back());
        let mut drawn_loops: Vec<track::Span> = Vec::new();
        let mut goes_back: Option<usize> = None;
        for laid in lanes.loops {
            let within = at_now.is_some_and(|at| (laid.first..=laid.last).contains(&at));
            let round = within
                .then(|| self.engine.0.rounds(laid.of))
                .and_then(|(now, of)| Some(format!("{}/{}", now + 1, of?)));
            let words = round.or_else(|| repeat_words(plan.flow_of(laid.of)?.repeat));
            let own = match laid.of {
                FlowOf::Span(id) => Some(id),
                _ => None,
            };
            let color = match own {
                // It was just not cut: it flashes, as the steps do.
                Some(id) if alarm.is_some() && self.alarm.1.contains(&id) => alarm_ink,
                // Letting go now would cut it.
                Some(id) if would_cut.contains(&id) => p.error,
                // Or take the carried steps out of it.
                Some(id) if would_leave.contains(&id) => Palette::tint(p.primary, 0.35),
                _ if back == Some(laid.of) => p.tertiary,
                _ => p.primary,
            };
            let mark = words.map_or(track::Mark::Nothing, track::Mark::Words);
            if let Some(span) = span(laid, color, mark, false) {
                if back == Some(laid.of) {
                    goes_back = Some(drawn_loops.len());
                }
                drawn_loops.push(span);
            }
        }
        let mut drawn_orders: Vec<track::Span> = lanes
            .orders
            .iter()
            .filter_map(|laid| span(laid, p.primary, track::Mark::Crossed, false))
            .collect();
        // Where carried steps would land, each has a dot's place; and a
        // loop, a drawn order or a join that is wholly among them comes
        // with them, drawn beside their places.
        if gapped && !stepwise.is_empty() {
            let ink = Palette::tint(gap_ink, 0.7);
            // With every flow stripped (Alt), the places show none: what
            // is seen is what letting go would give.
            let stripped = self.carry_how() == lantern_engine::Carry::Stripped;
            let joined = moving.map_or_else(Vec::new, |drag| plan.joined_along(&drag.carried));
            stops.extend(stepwise.iter().enumerate().map(|(n, id)| {
                // A carried step that goes on by itself goes on into the
                // one carried after it: they land together.
                let next = stepwise.get(n + 1).copied();
                let goes = !stripped && next.is_some() && joined.contains(id);
                track::Stop {
                    id: *id,
                    dot: track::Dot::Place(ink),
                    line: goes.then_some(track::Line::Plain(ink)),
                    next: next.filter(|_| goes),
                    back: None,
                }
            }));
            let brought = |laid: &Laid, mark: track::Mark| {
                let all = !stripped
                    && (laid.first..=laid.last)
                        .filter(|at| !plan.steps()[*at].is_section())
                        .all(|at| stepwise.contains(&id_at(at)));
                all.then(|| track::Span {
                    first: id_at(laid.first),
                    last: id_at(laid.last),
                    starts: true,
                    ends: true,
                    lane: laid.lane,
                    color: ink,
                    mark,
                    pulled: true,
                    shift: 0.0,
                    marks: Vec::new(),
                    pulse: None,
                })
            };
            for laid in lanes.loops {
                let words = plan
                    .flow_of(laid.of)
                    .and_then(|flow| repeat_words(flow.repeat));
                let mark = words.map_or(track::Mark::Nothing, track::Mark::Words);
                drawn_loops.extend(brought(laid, mark));
            }
            for laid in lanes.orders {
                drawn_orders.extend(brought(laid, track::Mark::Crossed));
            }
        }
        // What has played of the step that is on is shown along the line
        // it goes back by.
        if let Some(back) = goes_back
            && let Some(stop) = stops
                .iter_mut()
                .find(|stop| matches!(stop.dot, track::Dot::On { .. }))
            && let Some(track::Line::Playing { done, played, .. }) = stop.line
        {
            stop.back = Some((back, done, played));
        }
        // A line being pulled is drawn already, one lane out from what
        // it would hold; one that can't be, in the colour of what is
        // wrong.
        if let Some(pull) = &self.pull {
            use crate::rail::{Side, Would};
            let would = |first: usize, last: usize, among: &[Laid]| Laid {
                of: FlowOf::Plan,
                first,
                last,
                lane: among
                    .iter()
                    .filter(|held| first <= held.first && held.last <= last)
                    .map(|held| held.lane + 1)
                    .max()
                    .unwrap_or(0),
                section: None,
            };
            match pull.would {
                Some(Would::Loop { first, last }) => drawn_loops.extend(span(
                    &would(first, last, lanes.loops),
                    p.primary,
                    track::Mark::Nothing,
                    true,
                )),
                Some(Would::Shuffle { first, last }) => drawn_orders.extend(span(
                    &would(first, last, lanes.orders),
                    p.primary,
                    track::Mark::Crossed,
                    true,
                )),
                // The part that would be rubbed out, over the loop or
                // the order it is in.
                Some(Would::Rub { first, last }) if pull.side == Side::Right => drawn_orders
                    .extend(span(
                        &would(first, last, &[]),
                        p.error,
                        track::Mark::Nothing,
                        true,
                    )),
                Some(Would::Rub { first, last }) => {
                    let lane = pull
                        .line
                        .and_then(|line| {
                            lanes
                                .loops
                                .iter()
                                .find(|laid| laid.of == FlowOf::Span(line))
                        })
                        .map_or(0, |laid| laid.lane);
                    let rubbed = Laid {
                        lane,
                        ..would(first, last, &[])
                    };
                    drawn_loops.extend(span(&rubbed, p.error, track::Mark::Nothing, true));
                }
                Some(Would::Not { first, last, .. }) if pull.side == Side::Right => drawn_orders
                    .extend(span(
                        &would(first, last, lanes.orders),
                        p.error,
                        track::Mark::Nothing,
                        true,
                    )),
                Some(Would::Not { first, last, .. }) => drawn_loops.extend(span(
                    &would(first, last, lanes.loops),
                    p.error,
                    track::Mark::Nothing,
                    true,
                )),
                Some(Would::Join { .. }) | None => {}
            }
        }
        // The section the pointer is on, that what is carried would go
        // into, or whose foot is pulled is lit: a band from its header down
        // to its last row, or to where its foot would end it, and its line
        // strong. Those inside it are as they are, and the rest step back.
        let carrying = moving.is_some() || held.is_some();
        let lit = match (self.foot, carrying) {
            (Some((section, _)), _) => Some(section),
            (None, true) => landing.and(into.first().copied()),
            (None, false) => self
                .hovered
                .map(|(over, _)| over)
                .filter(|id| plan.get(*id).is_some_and(Step::is_section)),
        }
        .filter(|lit| {
            rows_listed
                .iter()
                .any(|row| matches!(row, Listed::Head { section, .. } if section == lit))
        });
        let band = lit.and_then(|section| {
            let pulled = self
                .foot
                .filter(|(foot, _)| *foot == section)
                .and_then(|(_, last)| last);
            let last = match pulled {
                Some(last) if plan.get(last).is_some_and(Step::is_section) => {
                    last_in(plan, &rows_listed, last, &into)
                }
                Some(last) => rows_listed
                    .iter()
                    .position(|row| *row == Listed::Step(last)),
                None => last_in(plan, &rows_listed, section, &into),
            }?;
            let last = match rows_listed[last] {
                Listed::Step(id) | Listed::Head { section: id, .. } => track::Key::Whole(id),
                Listed::Empty(id) => track::Key::Place(id),
                Listed::Gap => track::Key::Row(stepwise.last().copied().or(blockwise)?),
                Listed::Foot(_) => return None,
            };
            Some(track::Band {
                section,
                last,
                color: match foot_refused == Some(section) {
                    true => p.error,
                    false => hue(section),
                },
            })
        });
        // Each section's line beside its rows, in its colour: from where a
        // row says its line starts, or its dot, to its dot or its foot.
        let end_of = |row: Listed, top: bool| match row {
            Listed::Step(id)
            | Listed::Head {
                section: id,
                folded: true,
                ..
            } => Some(track::Key::Dot(id)),
            Listed::Head { section, .. } => Some(track::Key::Row(section)),
            Listed::Foot(section) => Some(track::Key::Foot(section)),
            Listed::Gap => gap_keys.map(|(first, last)| if top { first } else { last }),
            Listed::Empty(_) => None,
        };
        let stretched: Vec<track::Stretch> = stretches(plan, &rows_listed, &into)
            .into_iter()
            .filter_map(|(section, from, to)| {
                Some(track::Stretch {
                    from: end_of(rows_listed[from], true)?,
                    to: end_of(rows_listed[to], false)?,
                    color: match (foot_refused == Some(section), self.over(timing, section)) {
                        (true, _) => p.error,
                        (false, true) => Palette::tint(hue(section), 0.45),
                        (false, false) => hue(section),
                    },
                    light: lit_of(plan, lit, section),
                })
            })
            .collect();
        // A line that was pulled where none can be stays a moment, where
        // it was, shaking and flashing while why is said.
        if let (Some(aside), Some((pulled_to, first, last))) = (alarm, self.alarm_pull) {
            let refused = Laid {
                of: FlowOf::Plan,
                first,
                last,
                lane: 0,
                section: None,
            };
            if let Some(mut line) = span(&refused, alarm_ink, track::Mark::Nothing, true) {
                line.shift = aside;
                match pulled_to {
                    crate::rail::Side::Right => drawn_orders.push(line),
                    _ => drawn_loops.push(line),
                }
            }
        }
        // The open note's threads: from its card's edge to each thing it
        // is on, where those are now.
        let threads: Vec<(iced::Point, iced::Point)> = self
            .note_open
            .and_then(|id| plan.note(id))
            .map(|note| {
                let places = places.borrow();
                let Some(origin) = places.get(&track::Key::Canvas).copied() else {
                    return Vec::new();
                };
                // Each row it is on, and how far left of the row's end its
                // marks are: a step's left of its length, a section's at the
                // end of its header.
                let rows: Vec<(Rectangle, f32)> = note
                    .on
                    .iter()
                    .filter_map(|on| crate::notes::anchor(plan, *on))
                    .filter_map(|id| {
                        let from_end = match plan.get(id).is_some_and(Step::is_section) {
                            true => 17.0,
                            false => step_rows.marks_from_end(),
                        };
                        Some((places.get(&track::Key::Whole(id)).copied()?, from_end))
                    })
                    .collect();
                let Some((first, _)) = rows.iter().max_by(|a, b| a.0.y.total_cmp(&b.0.y)) else {
                    return Vec::new();
                };
                // From the card's top edge, near its mark, to where each
                // mark is.
                let card = iced::Point::new(
                    (origin.width - crate::notes::CARD - 4.0).max(0.0) + 14.0,
                    first.y + first.height - origin.y + 2.0,
                );
                rows.iter()
                    .map(|(row, from_end)| {
                        (
                            card,
                            iced::Point::new(
                                row.x + row.width - origin.x - from_end,
                                row.center_y() - origin.y + 7.0,
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        // A line to a step folded away runs to its section's stacked
        // dot, as a loop over it does.
        let shown_as = |id: StepId| {
            plan.position(id)
                .and_then(|at| plan.within(at).into_iter().find(|section| shut(*section)))
                .unwrap_or(id)
        };
        for stop in &mut stops {
            stop.next = stop
                .next
                .map(shown_as)
                .filter(|next| *next != shown_as(stop.id));
        }
        let picture = track::Picture {
            stops,
            loops: drawn_loops,
            orders: drawn_orders,
            still: self.motion.still,
            hot: can,
            stretches: stretched,
            band,
            threads,
            lit: match carrying {
                true => self.motion.host_lit(),
                false => 1.0,
            },
            carrying,
            sounds: plan
                .steps()
                .iter()
                .filter_map(|step| {
                    // Its end being pulled: where letting go would put it.
                    let until = match self.sound_end {
                        Some((owner, Some(to))) if owner == step.id => {
                            Some(to).filter(|to| *to != owner)
                        }
                        _ => step.sound.as_ref()?.until,
                    }?;
                    plan.get(until)?;
                    Some(track::SoundLine {
                        first: step.id,
                        last: until,
                        color: SOUND_LINE,
                        played: self.apart_played(step.id),
                    })
                })
                .collect(),
            select: (tree && can).then_some(p.primary),
            band_anywhere: tree && can && self.mods.shift(),
            rows: seen_rows,
            filling: self.filling.is_some(),
        };
        // Leaving the list means letting go would move nothing.
        let mut layers = stack![
            rows,
            track::over(picture, &places, Message::Marquee, Message::PressSoundEnd,)
        ]
        .width(Fill);
        // The open note's card, beside the first thing it is on, with a
        // thread to each of them.
        if let Some(card) = self.note_card(plan)
            && let Some(note) = self.note_open.and_then(|id| plan.note(id))
        {
            let places = places.borrow();
            let origin = places.get(&track::Key::Canvas).copied();
            let rows: Vec<Rectangle> = note
                .on
                .iter()
                .filter_map(|on| crate::notes::anchor(plan, *on))
                .filter_map(|id| places.get(&track::Key::Whole(id)).copied())
                .collect();
            if let (Some(origin), Some(first)) =
                (origin, rows.iter().max_by(|a, b| a.y.total_cmp(&b.y)))
            {
                // Just under the last thing it is on, so all of them stay
                // in sight with their marks.
                let x = (origin.width - crate::notes::CARD - 4.0).max(0.0);
                let y = (first.y + first.height - origin.y + 2.0).max(0.0);
                layers = layers.push(iced::widget::pin(card).x(x).y(y));
            }
        }
        mouse_area(
            scrolling(layers)
                .id(ROWS)
                .on_scroll(|seen| Message::PlanScrolled(seen.absolute_offset().y)),
        )
        .on_exit(Message::LeftPlan)
        .into()
    }

    /// What can be done to steps picked together: make a section of them,
    /// or a note on them.
    fn picked_actions(&self) -> Element<'_, Message> {
        let p = self.palette;
        let make = hinted(
            p,
            button(text("+ Section").size(META).font(BOLD).color(p.primary))
                .padding([3, 6])
                .on_press(Message::BracketPicked)
                .style(look::pill(p, Tone::Selected)),
            "Make a section of them".to_string(),
            tooltip::Position::Top,
        );
        // And a note on all of them.
        let note = hinted(
            p,
            button(look::icon(Icon::Note, 13.0, crate::notes::INK))
                .padding([4, 6])
                .on_press(Message::NoteFor(
                    self.picked
                        .iter()
                        .map(|id| lantern_engine::NoteOn::Step(*id))
                        .collect(),
                ))
                .style(look::pill(p, Tone::Selected)),
            "A note on them".to_string(),
            tooltip::Position::Top,
        );
        row![make, note].spacing(3).into()
    }

    /// What is inside a step, a line each under its row: the videos,
    /// sounds and pictures on a deck's slides, by slide, and the step's
    /// own sound with the slide it starts at. `on_stage`: the step is the
    /// one the service is at, so the slide it is on is marked. Each is in
    /// from its row's name, `inset` in from the row's start.
    fn step_parts<'a>(&self, step: &Step, on_stage: bool, inset: f32) -> Vec<Element<'a, Message>> {
        let p = self.palette;
        let indent = 34.0 + inset;
        let Action::Show { resource } = &step.action else {
            // A note or a live slot under a sound that keeps going says so.
            return crate::sources::mix_lines(&self.engine.0.plan(), step)
                .into_iter()
                .map(|(icon, words)| {
                    container(
                        row![
                            look::icon(icon, 12.0, p.on_surface_variant),
                            single_line(words).size(META).color(p.on_surface_variant),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .padding(iced::Padding::default().left(indent).top(2.0).bottom(3.0))
                    .into()
                })
                .collect();
        };
        let at = self
            .stage
            .picture()
            .filter(|scene| on_stage && scene.resource.id == resource.id)
            .map(lantern_engine::Scene::frame);
        let id = step.id;
        // Each is a way to its slide: in a run it goes on the projector,
        // while planning it is looked at here.
        let line = |icon: Icon, words: String, lit: bool, slide: usize| -> Element<'a, Message> {
            let colour = if lit {
                p.tertiary
            } else {
                p.on_surface_variant
            };
            button(
                row![
                    look::icon(icon, 12.0, colour),
                    container(single_line(words).size(META).color(colour))
                        .width(Fill)
                        .clip(true),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding(
                iced::Padding::default()
                    .left(indent)
                    .right(8.0)
                    .top(2.0)
                    .bottom(3.0),
            )
            .on_press(Message::StepSlide(id, slide))
            .style(look::row(p, Tone::Plain))
            .into()
        };
        let mut parts = Vec::new();
        for (slide, on_slide) in self
            .parts
            .get(&resource.id)
            .into_iter()
            .flatten()
            .enumerate()
        {
            for (n, part) in on_slide.iter().enumerate() {
                let (icon, name) = crate::dashboard::attachment_name(part, n, on_slide.len());
                parts.push(line(
                    icon,
                    format!("Slide {} · {name}", slide + 1),
                    at == Some(slide),
                    slide,
                ));
            }
        }
        // Its file changed since it was opened into pages: its first
        // page says so, and takes it in.
        if resource.page_of() == Some(0)
            && let Some(file) = resource.file_of_page()
            && self
                .files_changed
                .iter()
                .any(|changed| changed.id == file.id)
        {
            let reading = self.taking_in.as_ref() == Some(&file.id);
            let words = match reading {
                true => "The file changed. Reading it again…",
                false => "The file changed. Take it in",
            };
            parts.push(
                button(
                    row![
                        look::icon(Icon::Warning, 12.0, p.tertiary),
                        // Said in full, over two lines where the row is narrow.
                        text(words).size(META).color(p.tertiary).width(Fill),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .width(Fill)
                .padding(
                    iced::Padding::default()
                        .left(indent)
                        .right(8.0)
                        .top(2.0)
                        .bottom(3.0),
                )
                .on_press_maybe(
                    (!reading && self.can_edit()).then(|| Message::TakeIn(file.id.clone())),
                )
                .style(look::row(p, Tone::Plain))
                .into(),
            );
        }
        // Where its picture comes from, when not from what it is.
        for (icon, words) in crate::sources::mix_lines(&self.engine.0.plan(), step) {
            parts.push(line(icon, words, false, 0));
        }
        if let Some(sound) = &step.sound {
            let words = match sound.from {
                0 => format!("With {}", sound.resource.title),
                from => format!("Slide {} on · {}", from + 1, sound.resource.title),
            };
            parts.push(line(
                Icon::Music,
                words,
                at.is_some_and(|at| at >= sound.from),
                sound.from,
            ));
        }
        parts
    }

    /// `row`, telling the app which half of it the pointer is over, so a
    /// held row can be dropped above or below it. While a drag moves, the
    /// pointer is a closed hand.
    /// `row` with its ring on a right click, where the plan can change.
    fn ringed<'a>(
        &self,
        on: crate::rail::RingOn,
        row: Element<'a, Message>,
    ) -> Element<'a, Message> {
        match self.can_edit() {
            true => mouse_area(row)
                .on_right_press(Message::Rail(crate::rail::Act::Ring(on)))
                .into(),
            false => row,
        }
    }

    fn draggable<'a>(&self, id: StepId, row: Element<'a, Message>) -> Element<'a, Message> {
        let half = |half: Half| {
            mouse_area(Space::new().width(Fill).height(Fill))
                .on_enter(Message::Hover(Some((id, half))))
        };
        let area = mouse_area(stack![row, column![half(Half::Upper), half(Half::Lower)]])
            .on_exit(Message::Unhover(id));
        if self.drag.as_ref().is_some_and(|drag| drag.moving) {
            area.interaction(mouse::Interaction::Grabbing).into()
        } else if self.balk.is_some_and(|(_, pulled)| pulled) {
            // Pulled at in a run that won't let it move.
            area.interaction(mouse::Interaction::NotAllowed).into()
        } else {
            area.into()
        }
    }

    /// A step's row. With its `facts` it is tall, as while planning (see
    /// [`Rows`]); without, one line.
    pub(crate) fn step_row(
        &self,
        step: &Step,
        place: Place,
        issue: Option<&Issue>,
        facts: Option<&RowFacts<'_>>,
        noted: &[(lantern_engine::NoteId, usize)],
        picked_end: bool,
    ) -> Element<'_, Message> {
        let p = self.palette;
        let rows = match facts {
            Some(_) => Rows::Tall,
            None => Rows::Line,
        };
        // The current step is on stage unless something off the plan was
        // shown since.
        let shown = match &step.action {
            Action::Show { .. } => {
                self.stage
                    .program
                    .as_ref()
                    .is_some_and(|scene| crate::dashboard::shows(step, scene))
                    || self.apart_of(step.id).is_some()
            }
            _ => false,
        };
        let tone = match place {
            Place::Current if shown => Tone::Live,
            _ if self.selected == Some(step.id) || self.picked.contains(&step.id) => Tone::Selected,
            _ => Tone::Plain,
        };
        // On stage, the row says how far into it the stage is.
        // Its own sound playing apart says it, over a picture without an
        // end of its own.
        let apart = self.apart_of(step.id);
        let progress = match (place, apart, &self.stage.program) {
            (Place::Current, Some(bed), program)
                if program.as_ref().is_none_or(|scene| scene.timed().is_none()) =>
            {
                progress(&bed.media, &bed.resource)
            }
            (Place::Current, _, Some(scene)) if shown => progress(&scene.media, &scene.resource),
            _ => None,
        };
        // Just gone off, it fades from lit to played.
        let going_off = self
            .motion
            .going_off(step.id)
            .filter(|_| place == Place::Done)
            .map(crate::motion::ease);
        let label_color = match (place, tone) {
            (Place::Done, _) => match going_off {
                Some(t) => crate::motion::blend(Tone::Live.text(p), p.outline, t),
                None => p.outline,
            },
            (_, Tone::Plain) => p.on_surface,
            (_, tone) => tone.text(p),
        };
        // The name is edited where it's read.
        let title: Element<'_, Message> = if self.editing(step.id, Part::Name) {
            self.name_field(step)
        } else {
            single_line(step.label.clone())
                .size(BODY)
                .font(if tone == Tone::Plain { MEDIUM } else { BOLD })
                .color(label_color)
                .into()
        };
        // One short thing at the row's end, the most pressing there is:
        // what's wrong with the step, how far the stage is into it, when
        // it goes on by itself, what kind it is; else how long it takes.
        let length = self.step_length(step);
        // A download under way is a ring at the row's end, not words.
        let coming = match &step.action {
            Action::Show { resource } => self.download_part(resource),
            _ => None,
        };
        let kind = match &step.action {
            Action::Show { resource } => self.download_state(resource).filter(|_| coming.is_none()),
            Action::Live { hint, shown } => Some(slot_detail(hint.as_ref(), shown.len())),
            Action::Blank => Some("black".to_string()),
            Action::Note => Some("note".to_string()),
            Action::Section { .. } => None,
        }
        // "Note · note" says nothing twice: the kind shows once renamed.
        .filter(|kind| !kind.eq_ignore_ascii_case(step.label.trim()));
        let (said, said_color) = match (issue.filter(|_| coming.is_none()), place) {
            (Some(issue), _) => (
                Some(issue.what.clone()),
                level_tone(Some(issue.level)).text(p),
            ),
            (None, Place::Current) if self.goes_on_in().is_some() => (
                self.goes_on_in()
                    .map(|left| format!("goes on in {}", seconds_left(left))),
                label_color,
            ),
            (None, Place::Current) if shown => (
                progress.as_ref().map(|(how_far, _)| how_far.clone()),
                label_color,
            ),
            (None, Place::Current) if self.off_plan().is_some() => {
                (Some("waiting".to_string()), label_color)
            }
            (None, _) => (kind, p.on_surface_variant),
        };
        // While planning, the line under its name says what there is to
        // say.
        let said = said.filter(|_| facts.is_none());
        let selected =
            self.selected == Some(step.id) && !self.drag.as_ref().is_some_and(|drag| drag.moving);
        // The length is edited where it's read: on the selected row a click
        // on it turns it into its field. A live slot has none to set.
        let has_length = !matches!(step.action, Action::Live { .. });
        // In a service the step that is on says what there is to say under
        // its name, so its name keeps the row's width.
        let on_now = place == Place::Current && self.mode == crate::mode::Mode::Run;
        let end: Element<'_, Message> = if self.editing(step.id, Part::Length) {
            self.step_length_field(step)
        } else if on_now && said.is_some() {
            Space::new().into()
        } else if selected && has_length {
            let shown = if length.is_empty() {
                "min".to_string()
            } else {
                length.clone()
            };
            // Filled like a button, so it reads as something to click.
            button(ramped(
                step,
                text(shown).size(META).color(p.on_surface).into(),
                p.on_surface,
            ))
            .padding([2, 6])
            .on_press(Message::EditStep(Some((step.id, Part::Length))))
            .style(look::pill(p, Tone::Plain))
            .into()
        } else {
            // What there is to say, else the length. Whether it goes on
            // by itself is the rail's to show.
            let words = said.clone().unwrap_or_else(|| length.clone());
            let color = if said.is_some() {
                said_color
            } else {
                p.on_surface_variant
            };
            ramped(
                step,
                text(words)
                    .size(META)
                    .color(color)
                    .wrapping(iced::widget::text::Wrapping::None)
                    .into(),
                p.on_surface_variant,
            )
        };
        // While planning, beside its length, whether it goes on by itself.
        let end = match facts {
            Some(facts) => row![goes_on_mark(p, facts.goes_on), end]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
            None => end,
        };
        let mut label = column![title];
        if self.editing(step.id, Part::Search) {
            label = label.push(self.search_field(step));
        } else if let Some(facts) = facts {
            label = label.push(under_name(p, &facts.under));
        } else if (selected || on_now)
            && let Some(said) = &said
        {
            // The selected row says under its name what there is to say
            // of it: its end has the length to click.
            label = label.push(single_line(said.clone()).size(META).color(said_color));
        }
        // What can be done to the selected row comes up at its end while
        // the pointer is on it, in the length's place: at rest it is one
        // line like the others.
        let acting = selected && self.hovered.is_some_and(|(over, _)| over == step.id);
        // Its dot is on the rail beside it. One the phone put on the
        // projector says so here.
        let ring = self.motion.ring().filter(|_| shown);
        let mut line = row![].spacing(8).align_y(Alignment::Center);
        if let Some(facts) = facts {
            line = line.push(self.row_picture(&facts.thumb, PICTURE));
        }
        let mut line = line.push(container(label).width(Fill).clip(true));
        // Under the pointer, a row that can change says so, in its
        // length's place so its name keeps its room: what would replace
        // it, and the way to take it out. The selected row has all of
        // that on its second line already.
        let pointed = self.pointed(step.id);
        if pointed {
            line = line.push(self.pointed_actions(step.id));
        }
        if place == Place::Current && shown && self.motion.by_phone() && self.motion.rung_by_phone()
        {
            line = line.push(look::icon(Icon::Phone, 13.0, label_color));
        }
        // Its notes' marks: pressed, one opens. The step that is on in a
        // service has its notes open under it.
        for (id, n) in noted.iter().filter(|_| !on_now) {
            line = line.push(crate::notes::mark(*n, Some(Message::OpenNote(*id))));
        }
        // A step that comes again says so, and how often.
        if let Some(words) = repeat_words(step.flow.repeat) {
            line = line.push(
                row![
                    look::icon(Icon::Repeat, 13.0, p.primary),
                    text(words).size(META).font(BOLD).color(p.primary)
                ]
                .spacing(3)
                .align_y(Alignment::Center),
            );
        }
        // While planning, its picture says how far its download has come.
        let line = match coming.filter(|_| facts.is_none()) {
            Some(part) => line.push(hinted(
                p,
                look::ring(part, 16.0, p.tertiary),
                format!("Downloading: {:.0}%", part * 100.0),
                tooltip::Position::Top,
            )),
            None => line,
        };
        let line = match (acting, pointed) {
            (true, _) => line.push(self.step_actions(step)).push(end),
            (false, true) => line,
            (false, false) => line.push(end),
        };
        // Its row is lit a shade more as it goes up, and settles.
        let fill = look::row_fill(p, tone);
        let fill = match ring {
            Some(ring) => iced::Color {
                a: crate::motion::between((fill.a * 1.9).min(1.0), fill.a, ring),
                ..fill
            },
            None => fill,
        };
        let fill = match going_off {
            Some(t) => crate::motion::blend(look::row_fill(p, Tone::Live), fill, t),
            None => fill,
        };
        // Just added, replaced or dropped here: it flashes once, so the
        // eye finds where it went.
        let fill = match self.motion.flash(step.id) {
            Some(faded) => crate::motion::blend(Palette::tint(p.primary, 0.5), fill, faded),
            None => fill,
        };
        // Under all of its line, in line with its name: what can be done
        // to the steps picked together, under the last of them so its
        // name keeps the row's width; and picked in a run, its look and
        // timing.
        let inset = iced::Padding::default().left(rows.inset());
        let mut line = column![line];
        if picked_end {
            line = line.push(container(self.picked_actions()).padding(inset));
        }
        if selected && self.card_open() == Some((step.id, CardAt::Row)) {
            line =
                line.push(container(self.look_and_timing_card(step, CardAt::Row)).padding(inset));
        }
        let row = button(line)
            .width(Fill)
            .padding(rows.padding())
            .on_press(Message::Click(Clicked::Step(step.id)))
            .style(look::row_filled(p, fill));
        row.into()
    }

    /// A row's picture, `width` wide at 16:9, with its size in its lower
    /// right corner and its state in its upper left: white on a dark
    /// scrim, so they read over any picture, or a warning solid in its
    /// tint.
    pub(crate) fn row_picture<'a>(&self, thumb: &Thumb<'_>, width: f32) -> Element<'a, Message> {
        let picture = match thumb.picture {
            Picture::Of(resource) => self.thumbnail(resource, width),
            Picture::Kind(icon) => self.icon_thumbnail(icon, width, width * 9.0 / 16.0),
        };
        self.badged(picture, thumb, width, None, false)
    }

    /// A tile's picture in the library's Shapes: whole, `width` by
    /// `height`, which is its own shape, with its corners as a row's
    /// picture has them and a ○ in `primary` in its upper right one while
    /// it is `in_plan`. Over its foot, its `title`, where the tile has
    /// room for some of it beside its size.
    pub(crate) fn tile_picture<'a>(
        &self,
        thumb: &Thumb<'_>,
        (width, height): (f32, f32),
        title: &str,
        in_plan: bool,
    ) -> Element<'a, Message> {
        let picture = match thumb.picture {
            Picture::Of(resource) => self.whole_thumbnail(resource, width, height),
            Picture::Kind(icon) => self.icon_thumbnail(icon, width, height),
        };
        let size = thumb.size.as_deref().map_or(0.0, |words| {
            widgets::steady(words, badge_size(words, width)) + 2.0 * BADGE_PAD
        });
        let foot = crate::gallery::titled(width, size).then(|| {
            single_line(title)
                .size(META)
                .font(MEDIUM)
                .color(iced::Color::WHITE)
                .into()
        });
        self.badged(picture, thumb, width, foot, in_plan)
    }

    /// `picture`, `width` wide, with `thumb`'s size and state in its
    /// corners, and a ○ in `primary` in its upper right one while it is
    /// `in_plan`; and `foot`, where there is one, along its foot before
    /// the size, on a shade that darkens to the edge so white words read
    /// over any picture.
    fn badged<'a>(
        &self,
        picture: Element<'a, Message>,
        thumb: &Thumb<'_>,
        width: f32,
        foot: Option<Element<'a, Message>>,
        in_plan: bool,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let scrim = (iced::Color::BLACK.scale_alpha(0.74), iced::Color::WHITE);
        let badge = |words: String, round: f32, (fill, ink): (iced::Color, iced::Color)| {
            container(
                text(words.clone())
                    .size(badge_size(&words, width))
                    .font(BOLD)
                    .color(ink)
                    .wrapping(iced::widget::text::Wrapping::None),
            )
            .padding([0.0, BADGE_PAD])
            .style(move |_| container::Style {
                background: Some(fill.into()),
                border: iced::Border {
                    radius: round.into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            })
        };
        let size = thumb.size.clone().map(|size| badge(size, 3.0, scrim));
        let mut layers = stack![picture];
        match foot {
            Some(foot) => {
                let line = row![container(foot).width(Fill).clip(true)]
                    .extend(size.map(Into::into))
                    .spacing(4)
                    .align_y(Alignment::End);
                let shade = iced::gradient::Linear::new(iced::Radians::PI)
                    .add_stop(0.0, iced::Color::BLACK.scale_alpha(0.0))
                    .add_stop(1.0, iced::Color::BLACK.scale_alpha(0.7));
                layers = layers.push(
                    container(
                        container(line)
                            .width(Fill)
                            .padding(
                                iced::Padding::new(BADGE_INSET)
                                    .top(10.0)
                                    .left(BADGE_INSET + 4.0),
                            )
                            .style(move |_| container::Style {
                                background: Some(shade.into()),
                                border: iced::border::rounded(iced::border::bottom(
                                    look::THUMB_RADIUS,
                                )),
                                ..container::Style::default()
                            }),
                    )
                    .align_bottom(Fill),
                );
            }
            None => {
                if let Some(size) = size {
                    layers = layers.push(
                        container(size)
                            .align_right(Fill)
                            .align_bottom(Fill)
                            .padding(BADGE_INSET),
                    );
                }
            }
        }
        if let Some(state) = thumb.state {
            let colours = match state {
                State::Coming(_) | State::Online => scrim,
                State::Live => (scrim.0, p.tertiary),
                State::Soft(_) => (p.tertiary, p.surface),
            };
            let badge: Element<'a, Message> = match state {
                // Its words say why, where the line under it doesn't.
                State::Soft(screen) => hinted(
                    p,
                    badge(state.words(), look::PILL_RADIUS, colours),
                    soft_words(screen),
                    tooltip::Position::Bottom,
                ),
                State::Live | State::Online | State::Coming(_) => {
                    badge(state.words(), look::PILL_RADIUS, colours).into()
                }
            };
            layers = layers.push(
                container(badge)
                    .align_left(Fill)
                    .align_top(Fill)
                    .padding(BADGE_INSET),
            );
        }
        if in_plan {
            layers = layers.push(
                container(badge(
                    "○".to_string(),
                    look::PILL_RADIUS,
                    (scrim.0, p.primary),
                ))
                .align_right(Fill)
                .align_top(Fill)
                .padding(BADGE_INSET),
            );
        }
        layers.into()
    }

    /// The same row while the service runs: how long it has run, large;
    /// the time it is held against; how far it runs late or early; when
    /// it is now expected to end; and the bar, grey where a section is
    /// over, the stage's colour where the service is.
    fn run_time_row<'a>(&self, plan: &Plan, timing: &Timing) -> Element<'a, Message> {
        let p = self.palette;
        let now = SystemTime::now();
        let since = |then: SystemTime| now.duration_since(then).unwrap_or_default();
        let elapsed = self.engine.0.started().map(since).unwrap_or_default();
        let left = timing
            .end
            .map(|end| end.duration_since(now).unwrap_or_default());
        let scheduled =
            lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step)).total;
        let against = plan.target.unwrap_or(scheduled);
        let mut top = row![
            look::icon(Icon::Clock, 16.0, p.tertiary),
            text(length::clock(elapsed))
                .size(22)
                .font(BOLD)
                .color(p.on_surface)
                .width(widgets::steady(&length::clock(elapsed), 22.0)),
            container(
                text(format!("◎ {}", length::clock(against)))
                    .size(META)
                    .font(BOLD)
                    .color(p.primary)
            )
            .padding([3, 8])
            .style(look::chip(p, Tone::Plain)),
            Space::new().width(Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(left) = left {
            // Late or early: against the start time when there is one,
            // else against the time the service has.
            let late = match plan.starts {
                Some(starts) => {
                    lantern_engine::timing::behind(starts, scheduled, length::now_of_day(), left)
                }
                None => (elapsed + left).as_secs() as i64 - against.as_secs() as i64,
            };
            if late.abs() >= 30 {
                let (sign, colour) = if late > 0 {
                    ("↑", p.tertiary)
                } else {
                    ("↓", p.primary)
                };
                let late = format!(
                    "{sign} {}",
                    length::clock(Duration::from_secs(late.unsigned_abs()))
                );
                top = top.push(
                    text(late.clone())
                        .size(META)
                        .font(BOLD)
                        .color(colour)
                        .width(widgets::steady(&late, META)),
                );
            }
            let ends = format!("→ {}", length::clock_of_day(length::now_of_day() + left));
            top = top.push(
                text(ends.clone())
                    .size(META)
                    .color(p.on_surface_variant)
                    .width(widgets::steady(&ends, META)),
            );
        }
        let parts = timing
            .sections
            .iter()
            .filter(|section| {
                plan.get(section.id)
                    .is_some_and(|step| step.depth() == Some(0))
            })
            .map(|section| {
                let part = |of: Duration, whole: Duration| match whole.is_zero() {
                    true => 1.0,
                    false => of.as_secs_f32() / whole.as_secs_f32(),
                };
                let (length, full, colour) = match section.phase {
                    Phase::Done { took } => {
                        (section.budget.or(took).unwrap_or_default(), 1.0, p.outline)
                    }
                    Phase::Running { elapsed, expected } => {
                        let whole = section.budget.unwrap_or(expected);
                        (whole, part(elapsed, whole), p.tertiary)
                    }
                    Phase::Coming { planned } => {
                        (section.budget.unwrap_or(planned), 0.0, p.primary)
                    }
                };
                container(bar(p, full, colour, 0.0))
                    .width(Length::FillPortion(length.as_secs().clamp(1, 60_000) as u16))
                    .into()
            });
        column![top, row(parts).spacing(3)]
            .spacing(8)
            .padding(iced::Padding::default().left(8.0).right(8.0).bottom(8.0))
            .into()
    }

    /// What the plan takes: what its steps add up to, sections' own times
    /// left out, so it is what has been put in so far; that with
    /// sections' times; and whether something comes again with no end.
    fn plan_sums(&self, plan: &Plan) -> PlanSums {
        let length = |step: &Step| self.loaded_length(step);
        let total = lantern_engine::timing::schedule(plan, &length).total;
        let mut bare = plan.clone();
        let sections: Vec<StepId> = bare
            .steps()
            .iter()
            .filter(|step| step.is_section())
            .map(|step| step.id)
            .collect();
        for id in sections {
            if let Some(Step {
                action: Action::Section { budget, .. },
                ..
            }) = bare.get_mut(id)
            {
                *budget = None;
            }
        }
        let filled = lantern_engine::timing::schedule(&bare, &length);
        PlanSums {
            filled: filled.total,
            total,
            open: filled.open,
        }
    }

    /// The plan's own fields in its settings, each with what it is
    /// called: its name, the time it is meant to take, what its steps
    /// come to against that, when it starts and so ends, and how it
    /// plays. Each but what it takes is pressed to change it.
    pub(crate) fn plan_fields(&self, plan: &Plan) -> [(&'static str, Element<'_, Message>); 5] {
        let p = self.palette;
        let quiet = p.on_surface_variant;
        let can = self.can_edit();
        let name: Element<'_, Message> = match &self.plans.mode {
            plans::Mode::Renaming(draft) => text_input("Plan name", draft)
                .id(NAME_BOX)
                .on_input(Message::PlanName)
                .on_submit(Message::SubmitPlanName)
                .size(META)
                .padding([5, 9])
                .style(look::input(p))
                .into(),
            _ => button(
                single_line(plan.title.clone())
                    .size(META)
                    .font(BOLD)
                    .color(p.primary),
            )
            .width(Fill)
            .padding([5, 9])
            .on_press_maybe(can.then_some(Message::EditPlanName))
            .style(look::pill(p, Tone::Plain))
            .into(),
        };
        let small = |label: &'static str, does: Message| -> Element<'_, Message> {
            button(text(label).size(META).font(BOLD).color(p.on_surface))
                .padding([1, 4])
                .on_press_maybe(can.then_some(does))
                .style(look::pill(p, Tone::Plain))
                .into()
        };
        let pressed = |words: String, does: Message| -> Element<'_, Message> {
            button(text(words).size(META).font(BOLD).color(p.on_surface))
                .padding([2, 6])
                .on_press_maybe(can.then_some(does))
                .style(look::pill(p, Tone::Plain))
                .into()
        };
        let field = |hint: &'static str, id: &'static str, typed: &str| {
            text_input(hint, typed)
                .id(id)
                .size(META)
                .width(62)
                .padding([2, 6])
                .style(look::input(p))
        };
        // The time it is meant to take: a minute less or more each press,
        // or typed.
        let goal: Element<'_, Message> = match &self.target_typed {
            Some(typed) => field("45", TARGET_BOX, typed)
                .width(GOAL_WIDE)
                .on_input(Message::PlanTarget)
                .on_submit(Message::TargetDone)
                .into(),
            // As wide as the field, and the same whatever it says, so the
            // + beside it stays where it was pressed.
            None => button(
                text(plan.target.map_or("—".to_string(), length::clock))
                    .size(META)
                    .font(BOLD)
                    .color(p.on_surface)
                    .center(),
            )
            .width(GOAL_WIDE)
            .padding([2, 0])
            .on_press_maybe(can.then_some(Message::EditTarget))
            .style(look::pill(p, Tone::Plain))
            .into(),
        };
        let goal = row![
            small("−", Message::PlanGoal(-60)),
            goal,
            small("+", Message::PlanGoal(60)),
        ]
        .spacing(3)
        .align_y(Alignment::Center);
        // What its steps come to, and how far from the time it is meant
        // to take: over in the colour of what is wrong, under quietly.
        let sums = self.plan_sums(plan);
        let against = sums.against(plan.target);
        let over = against.is_some_and(|against| sums.filled > against);
        let mut sum = row![text(sums.said()).size(META).font(BOLD).color(if over {
            p.error
        } else {
            p.on_surface
        })]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(target) = against.filter(|against| *against != sums.filled) {
            let (sign, by, ink) = match over {
                true => ("+", sums.filled - target, p.error),
                false => ("−", target - sums.filled, quiet),
            };
            sum = sum.push(
                text(format!("{sign}{}", length::clock(by)))
                    .size(META)
                    .color(ink),
            );
        }
        // When it starts, typed, and so when it should end.
        let starts: Element<'_, Message> = match &self.starts_typed {
            Some(typed) => field("10:30", STARTS_BOX, typed)
                .on_input(Message::PlanStarts)
                .on_submit(Message::StartsDone)
                .into(),
            None => pressed(
                plan.starts
                    .map_or("—".to_string(), |starts| starts.to_string()),
                Message::EditStarts,
            ),
        };
        let mut starts = row![starts].spacing(8).align_y(Alignment::Center);
        if let Some(ends) = sums.ends(plan.starts, plan.target) {
            starts = starts.push(text(format!("→ {ends}")).size(META).color(quiet));
        }
        [
            ("Name", name),
            ("Goal", goal.into()),
            ("Takes", sum.into()),
            ("Starts", starts.into()),
            ("Plays", self.flow_buttons(p, None, plan.flow, true)),
        ]
    }

    pub(crate) fn flow_buttons<'a>(
        &self,
        p: Palette,
        of: Option<StepId>,
        flow: Flow,
        all: bool,
    ) -> Element<'a, Message> {
        let goes = flow.joined;
        let can = self.can_edit();
        let glyph = |icon: Icon, on: bool| {
            look::icon(
                icon,
                14.0,
                if on { p.primary } else { p.on_surface_variant },
            )
        };
        let small = |content: Element<'a, Message>, on: bool, about: &str, message: Message| {
            hinted(
                p,
                button(content)
                    .padding([3, 6])
                    .on_press_maybe(can.then_some(message))
                    .style(look::row(p, if on { Tone::Selected } else { Tone::Plain })),
                about.to_string(),
                tooltip::Position::Top,
            )
        };
        let mut buttons = row![].spacing(2).align_y(Alignment::Center);
        {
            buttons = buttons.push(small(
                glyph(if goes { Icon::Joined } else { Icon::Pause }, goes).into(),
                goes,
                if goes {
                    "Its steps go straight on. Click to make them wait"
                } else {
                    "Its steps wait for you. Click to make them go straight on"
                },
                Message::SetJoined(FlowOf::from(of), !goes),
            ));
        }
        let repeats = flow.repeat != Repeat::Once;
        if repeats || all {
            let mut content = row![glyph(Icon::Repeat, repeats)]
                .spacing(4)
                .align_y(Alignment::Center);
            // In a service, a number of times says which one this is.
            let words = match flow.repeat {
                Repeat::Times(times) if self.mode == crate::mode::Mode::Run => {
                    let round = self.engine.0.round_of(of).saturating_add(1).min(times);
                    Some(format!("{round} / {times}"))
                }
                repeat => repeat_words(repeat),
            };
            if let Some(words) = words {
                content = content.push(text(words).size(META).font(BOLD).color(p.primary));
            }
            // Until a time, it says how far past the time it ends.
            if let Repeat::Until(time) = flow.repeat {
                let plan = self.engine.0.plan();
                let over = lantern_engine::timing::lands(&plan, of, time, flow.ends, &|step| {
                    self.loaded_length(step)
                })
                .and_then(|lands| lands.ends)
                .map(|ends| ends.over)
                .filter(|over| *over >= Duration::from_secs(30));
                if let Some(over) = over {
                    content = content.push(
                        text(format!("+{}", (over.as_secs() + 30) / 60))
                            .size(META)
                            .font(BOLD)
                            .color(p.tertiary),
                    );
                }
            }
            buttons = buttons.push(small(
                content.into(),
                repeats,
                if repeats {
                    "It comes again. Click for once; right click for how often"
                } else {
                    "It comes once. Click to make it come again"
                },
                Message::SetRepeat(
                    of.into(),
                    if repeats {
                        Repeat::Once
                    } else {
                        Repeat::Times(2)
                    },
                ),
            ));
        }
        if flow.shuffle || all {
            buttons = buttons.push(small(
                glyph(Icon::Shuffle, flow.shuffle).into(),
                flow.shuffle,
                if flow.shuffle {
                    "Its steps come in a drawn order. Click for the plan's order"
                } else {
                    "Its steps come in the plan's order. Click to shuffle them"
                },
                Message::SetShuffle(of.into(), !flow.shuffle),
            ));
        }
        // Rehearse it in the preview; while it is, the way to stop.
        let rehearsing = self.rehearsal.as_ref().is_some_and(|r| r.of == of);
        if self.mode == crate::mode::Mode::Plan {
            buttons = buttons.push(hinted(
                p,
                button(look::icon(
                    if rehearsing { Icon::Stop } else { Icon::Play },
                    14.0,
                    if rehearsing {
                        p.on_primary
                    } else {
                        p.on_surface
                    },
                ))
                .padding([3, 6])
                .on_press(Message::Rehearse(of))
                .style(look::pill(
                    p,
                    if rehearsing { Tone::Solid } else { Tone::Plain },
                )),
                if rehearsing {
                    "Stop rehearsing".to_string()
                } else {
                    "Rehearse it in the preview: nothing goes on the projector".to_string()
                },
                tooltip::Position::Top,
            ));
        }
        buttons.into()
    }

    /// The rail beside the step at `index`: its dot, the line down to
    /// it from the step before when that one goes on by itself, and the
    /// line on to the next when this one does. A gap is a step that
    /// waits. Planned is `planned`, the colour of the innermost section
    /// the step is in (the plan's in none), live the stage's, past grey;
    /// under the step on the projector the line fills as it plays.
    /// A click under the dot joins the step to the next, or parts them.
    fn rail<'a>(
        &self,
        plan: &Plan,
        index: usize,
        place: Place,
        planned: iced::Color,
        lanes: Lanes<'_>,
        places: &track::Places,
    ) -> (Element<'a, Message>, track::Stop) {
        let p = self.palette;
        let steps = plan.steps();
        let step = &steps[index];
        let goes = plan.goes_on(index);
        let before = steps[..index].iter().rposition(|step| !step.is_section());
        let after = steps[index + 1..].iter().any(|step| !step.is_section());
        // The step its line goes to: the next one, whatever section
        // that is in.
        let next = plan.next_step(index).map(|next| &steps[next]);
        let joined_above = before.is_some_and(|before| plan.goes_on(before));
        let joined_below = goes && after;
        // Past is grey in a service. In a rehearsal it is the plan's
        // colour, dimmed: nothing there has really happened.
        let past = match self.rehearsal {
            Some(_) => Palette::tint(planned, 0.5),
            None => p.outline,
        };
        // A rehearsal is the plan's colour: nothing of it is live.
        let rehearsed = self.rehearsed_part(step.id);
        let (place, live) = match rehearsed {
            Some(_) => (Place::Current, planned),
            None => (place, p.tertiary),
        };
        // How far the stage is into this step, when it is the one up.
        let on_stage = |step: &Step| match (&step.action, &self.stage.program) {
            (Action::Show { resource }, Some(scene)) if scene.resource.id == resource.id => {
                Some(progress(&scene.media, &scene.resource).map_or(0.0, |(_, done)| done))
            }
            _ => None,
        };
        let played = match place {
            Place::Current => on_stage(step),
            _ => None,
        }
        .or(rehearsed);
        // The one that is up has a ring round its dot, which takes its
        // room from the line above: the dot stays where it was.
        let on = place == Place::Current;
        // "Next" is the service's word: while planning, a step is only
        // one that goes on or one that waits.
        let place = match place {
            Place::Next if self.mode != crate::mode::Mode::Run => Place::Later,
            place => place,
        };
        // A dot held down and pulled to another step's shows what letting
        // go would do: the rows it is about are lit, with the loop back
        // up or the line down drawn already. Where no loop can be, in
        // the colour of what is wrong.
        let would = self
            .pull
            .as_ref()
            .and_then(|pull| pull.would)
            .filter(|would| {
                let (first, last) = would.rows();
                (first..=last).contains(&index)
            });
        let would_ink = match would {
            Some(crate::rail::Would::Not { .. }) => p.error,
            Some(crate::rail::Would::Rub { .. }) => p.on_surface_variant,
            _ => planned,
        };
        // How the dot looks: drawn by the one drawing over the list.
        let look = match place {
            _ if would.is_some() => track::Dot::Would(would_ink),
            // Up now: its dot, in a ring that is always there.
            Place::Current => track::Dot::On {
                color: live,
                ring: Palette::tint(live, 0.5),
                glow: self.motion.ring().filter(|_| played.is_some()),
            },
            Place::Next => track::Dot::Hollow(live),
            Place::Done => track::Dot::Solid(past),
            Place::Later if goes => track::Dot::Solid(planned),
            Place::Later => track::Dot::Hollow(planned),
        };
        // The line from this dot to the next step's.
        let joining = matches!(would, Some(crate::rail::Would::Join { first, last }) if first <= index && index < last);
        let line = match (joined_below, place, played) {
            _ if joining => Some(track::Line::Plain(planned)),
            // The step that is on fills its line as it plays, whether or
            // not it goes on by itself: what is still to come is the
            // line as it would be, where it goes on into the next; a
            // quiet one where it waits.
            (_, Place::Current, Some(done)) => Some(track::Line::Playing {
                done: done.clamp(0.0, 1.0),
                played: live,
                ahead: match (rehearsed, joined_below) {
                    (Some(_), _) => Palette::tint(live, 0.35),
                    (None, true) => planned,
                    (None, false) => Palette::tint(p.on_surface_variant, 0.5),
                },
                thick: match rehearsed {
                    Some(_) => 1.0,
                    None => self.media.levels(&self.stage).left.unwrap_or(1.0),
                },
            }),
            (false, ..) => None,
            (true, Place::Done, _) => Some(track::Line::Plain(past)),
            // On, with nothing playing yet: the line as planned. The
            // stage's colour on it always means that much has played.
            (true, ..) => Some(track::Line::Plain(planned)),
        };
        let stop = track::Stop {
            id: step.id,
            dot: look,
            line,
            next: next.map(|next| next.id),
            back: None,
        };
        // What is here in the row is only room to press: the dot, the
        // line over it and the line under it.
        let above = Space::new().width(RAIL_LINE).height(match on {
            true => lanes.rows.dot() - RAIL_RING,
            false => lanes.rows.dot(),
        });
        let size = match on {
            true => RAIL_DOT + 2.0 * RAIL_RING,
            false => RAIL_DOT,
        };
        let dot = track::seen(
            track::Key::Dot(step.id),
            places,
            Space::new().width(size).height(size),
        );
        let below = Space::new().width(RAIL_LINE).height(Fill);
        // The dot and the line under it are one thing to press: what is
        // still ahead can be joined to the next step or parted from it,
        // where the plan can change. The line over the dot is the join
        // from the step before, and pressing it changes that one.
        let can_change = |step: &Step| {
            self.can_edit()
                && goes_on_when(step).is_some()
                && (self.mode != crate::mode::Mode::Run || self.ahead(plan, step.id))
        };
        let pressed = |part: Element<'a, Message>, of: &Step, goes: bool| -> Element<'a, Message> {
            tooltip(
                mouse_area(part)
                    .on_press(Message::SetJoined(FlowOf::Step(of.id), !goes))
                    .interaction(mouse::Interaction::Pointer),
                container(
                    text(if goes {
                        "Goes straight on. Click to make it wait"
                    } else {
                        "Waits for you. Click to make it go straight on"
                    })
                    .size(META),
                )
                .padding([4, 8])
                .style(look::tip(p)),
                tooltip::Position::Right,
            )
            .delay(look::TIP_DELAY)
            .into()
        };
        let ahead = matches!(place, Place::Next | Place::Later);
        // A right click on the dot or on the line opens the ring of
        // what that one can do, where the plan can change.
        let ring = |on: crate::rail::RingOn| Message::Rail(crate::rail::Act::Ring(on));
        let rings = self.can_edit();
        let dot: Element<'a, Message> = match rings {
            true => mouse_area(dot)
                .on_right_press(ring(crate::rail::RingOn::Dot(step.id)))
                .into(),
            false => dot,
        };
        let below: Element<'a, Message> = container(below)
            .width(RAIL)
            .height(Fill)
            .center_x(RAIL)
            .into();
        let below: Element<'a, Message> = match rings && after {
            true => mouse_area(below)
                .on_right_press(ring(crate::rail::RingOn::Line(step.id)))
                .into(),
            false => below,
        };
        let lower: Element<'a, Message> = column![dot, below]
            .width(RAIL)
            .height(Fill)
            .align_x(Alignment::Center)
            .into();
        // A click on it makes the step go on or wait, where that may
        // change. Held and pulled to another step's dot, it draws a loop
        // or joins the steps between: while planning only.
        let toggle = (ahead && after && can_change(step)).then_some(!goes);
        let pulls = self.mode == crate::mode::Mode::Plan && self.can_edit();
        let press = Message::Rail(crate::rail::Act::Press {
            step: step.id,
            toggle,
        });
        // It says what a click does only while nothing is held or open.
        let quiet = self.pull.is_some() || self.ring.is_some();
        let lower: Element<'a, Message> = match (toggle, pulls) {
            (None, false) => lower,
            (Some(_), _) if quiet => mouse_area(lower)
                .on_press(press)
                .interaction(mouse::Interaction::Pointer)
                .into(),
            (None, true) => mouse_area(lower)
                .on_press(press)
                .interaction(mouse::Interaction::Pointer)
                .into(),
            (Some(_), _) => tooltip(
                mouse_area(lower)
                    .on_press(press)
                    .interaction(mouse::Interaction::Pointer),
                container(
                    text(if goes {
                        "Goes straight on. Click to make it wait"
                    } else {
                        "Waits for you. Click to make it go straight on"
                    })
                    .size(META),
                )
                .padding([4, 8])
                .style(look::tip(p)),
                tooltip::Position::Right,
            )
            .delay(look::TIP_DELAY)
            .into(),
        };
        let upper: Element<'a, Message> = container(above).center_x(RAIL).into();
        let upper = match before.map(|before| &steps[before]) {
            Some(before) if joined_above && ahead && can_change(before) => {
                pressed(upper, before, true)
            }
            _ => upper,
        };
        let upper: Element<'a, Message> = match before.map(|before| steps[before].id) {
            Some(before) if rings => mouse_area(upper)
                .on_right_press(ring(crate::rail::RingOn::Line(before)))
                .into(),
            _ => upper,
        };
        let id = step.id;
        let rail = column![upper, lower].width(RAIL).height(Fill);
        // Beside it, room for the lines of the spans it is in: what comes
        // again to the left, a drawn order to the right. Every row keeps
        // it, so the dots stay in one line down the plan. A right click
        // on a line there opens what is that span's own.
        let pulling = would.is_some();
        let covers = |laid: &Laid| (laid.first..=laid.last).contains(&index);
        let curl = self.loop_lanes(lanes, &covers, pulling, Some(step.id));
        let drawn = self.order_lanes(lanes, &covers, pulling, Some(step.id));
        // The rail says when the pointer is on it, for a dot pulled here
        // and for a ring opened here, and where it is, for what is drawn
        // over it. A width of its own: laid out after the step beside
        // it, a row that takes what it needs would be left none.
        let rail = mouse_area(track::seen(
            track::Key::Row(id),
            places,
            row![curl, rail, drawn]
                .width(lanes.curl + RAIL + lanes.drawn)
                .height(Fill),
        ))
        .on_enter(Message::Rail(crate::rail::Act::Over(id)))
        .on_exit(Message::Rail(crate::rail::Act::Left(id)))
        .into();
        (rail, stop)
    }

    /// The line of a section's header: its name in `color`, as quiet as
    /// what has played once it is over in a service, and what it takes
    /// against the time it is given, with a gauge while planning and in
    /// its pace's tint in a service; or, heading the service's `round`th
    /// time round it (from 0), which round. Folded, it says how many steps
    /// it holds. By its arrow, there while folded or pointed at, it folds
    /// or opens.
    pub(crate) fn head_line<'a>(
        &self,
        plan: &Plan,
        timing: &Timing,
        index: usize,
        color: iced::Color,
        folded: bool,
        round: u16,
    ) -> iced::widget::Row<'a, Message> {
        let p = self.palette;
        let step = &plan.steps()[index];
        let id = step.id;
        let quiet = p.on_surface_variant;
        // Over in a service: as quiet as what has played.
        let past = self.over(timing, id);
        let hue = match past {
            true => Palette::tint(color, 0.45),
            false => color,
        };
        let budget = match step.action {
            Action::Section { budget, .. } => budget.filter(|budget| !budget.is_zero()),
            _ => None,
        };
        let section = timing.sections.iter().find(|section| section.id == id);
        let phase = section.map_or(
            Phase::Coming {
                planned: self.planned(plan, id),
            },
            |section| section.phase,
        );
        let (taken, expected) = match phase {
            Phase::Coming { planned } => (planned, planned),
            Phase::Running { elapsed, expected } => (elapsed, expected),
            Phase::Done { took } => (took.unwrap_or_default(), took.unwrap_or_default()),
        };
        let empty = plan.group_end(index) == index + 1;
        let running = self.mode == crate::mode::Mode::Run;
        let words = time_words(
            taken,
            budget,
            step.done_by(),
            empty,
            running && matches!(phase, Phase::Done { .. }),
        );
        let late = budget.is_some_and(|budget| expected > budget)
            || section.and_then(SectionTiming::late).is_some();
        // Just tipped over its time: its words turn red as they go.
        let ink = match (late, self.motion.tipped(id)) {
            (true, Some(t)) => crate::motion::blend(quiet, p.error, t),
            (true, None) => p.error,
            (false, _) if past => p.outline,
            (false, _) => quiet,
        };
        // In a service, a pace that is not on track says so, in its tint.
        let pace = budget
            .filter(|_| running && !past && !empty)
            .map(|budget| Pace::of(expected, budget));
        let said: Element<'a, Message> = match (round, pace) {
            (0, Some(pace @ (Pace::Tight | Pace::Over))) => {
                let tone = pace_tone(pace);
                let word = match pace {
                    Pace::Over => "over",
                    _ => "tight",
                };
                container(
                    single_line(format!("{word} · {words}"))
                        .size(META)
                        .font(BOLD)
                        .color(tone.text(p)),
                )
                .padding([1, 8])
                .style(look::chip(p, tone))
                .into()
            }
            (0, _) => single_line(words).size(META).color(ink).into(),
            (round, _) => single_line(format!("round {}", round + 1))
                .size(META)
                .color(quiet)
                .into(),
        };
        let pointed = self.hovered.is_some_and(|(over, _)| over == id);
        let mut title = row![
            single_line(step.label.clone())
                .size(NAME)
                .font(BOLD)
                .color(hue)
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if folded || pointed {
            title = title.push(track::seen(
                track::Key::Chevron(id),
                &self.places,
                button(text(if folded { "▸" } else { "▾" }).size(BODY).color(hue))
                    .padding([0, 4])
                    .on_press(Message::Fold(id))
                    .style(look::row(p, Tone::Plain)),
            ));
        }
        if folded {
            let steps = (index + 1..plan.group_end(index))
                .filter(|at| !plan.steps()[*at].is_section())
                .count();
            title = title.push(single_line(count(steps, "step")).size(META).color(quiet));
        }
        let mut line = row![container(title).width(Fill).clip(true), said]
            .spacing(8)
            .align_y(Alignment::Center);
        // While planning, how full it is against its time.
        if let Some(budget) = budget.filter(|_| Rows::of(self.mode) == Rows::Tall && !empty) {
            let (full, over) = part_fill(Some(budget), taken);
            let color = match over {
                true => p.error,
                false => hue,
            };
            line = line.push(container(bar(p, full, color, 0.0)).width(GAUGE));
        }
        for (note, n) in crate::notes::numbers_on(plan, lantern_engine::NoteOn::Step(id)) {
            line = line.push(crate::notes::mark(n, Some(Message::OpenNote(note))));
        }
        line
    }

    /// A section's header, over what it holds: its line, `depth` levels
    /// in. Folded, it stands for all it holds: they are a stacked dot on
    /// the line. Its words start where the steps' pictures do, so the
    /// lines down the dots pass them by. Pressed it is picked, held and
    /// moved it is carried, twice or by its arrow it folds or opens, and a
    /// right click opens its ring.
    #[allow(clippy::too_many_arguments)]
    fn header<'a>(
        &self,
        plan: &Plan,
        timing: &Timing,
        index: usize,
        depth: u8,
        folded: bool,
        color: iced::Color,
        lanes: Lanes<'_>,
        places: &track::Places,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let rows = lanes.rows;
        let id = plan.steps()[index].id;
        let past = self.over(timing, id);
        let line = self.head_line(plan, timing, index, color, folded, 0);
        let picked = self.selected == Some(id) || self.picked.contains(&id);
        let fill = look::row_fill(
            p,
            match picked {
                true => Tone::Selected,
                false => Tone::Plain,
            },
        );
        let (room, tall) = (rows.head_room(), HEAD_LINE);
        let body = button(
            container(line)
                .padding(iced::Padding::default().left(INDENT * f32::from(depth)))
                .center_y(tall),
        )
        .width(Fill)
        .padding([0, 10])
        .on_press_maybe((!past).then_some(Message::Click(Clicked::Step(id))))
        .style(look::row_filled(p, fill));
        // In the dots' column: folded, its stacked dot, level with its
        // name; and from there down, where its section's line starts.
        let middle = room + tall / 2.0;
        let rail = match folded {
            true => column![
                Space::new().height(middle - RAIL_DOT / 2.0),
                track::seen(
                    track::Key::Dot(id),
                    places,
                    Space::new().width(RAIL_DOT).height(RAIL_DOT)
                ),
            ],
            false => column![Space::new().height(middle)],
        };
        let rail = rail
            .push(track::seen(
                track::Key::Row(id),
                places,
                Space::new().width(RAIL).height(Fill),
            ))
            .width(RAIL)
            .height(Fill)
            .align_x(Alignment::Center);
        // Its top, pressed and pulled while planning, starts the section
        // somewhere else.
        let above: Element<'a, Message> =
            match self.can_edit() && self.mode == crate::mode::Mode::Plan && !folded {
                true => column![
                    mouse_area(Space::new().width(Fill).height(FOOT))
                        .on_press(Message::PressHead(id))
                        .interaction(mouse::Interaction::ResizingVertically),
                    Space::new().height(room - FOOT),
                ]
                .into(),
                false => Space::new().height(room).into(),
            };
        // The lines of the spans it is in pass it, with their rings.
        let covers = |laid: &Laid| (laid.first..=laid.last).contains(&index);
        let header = row![
            Space::new().width(lanes.time),
            self.loop_lanes(lanes, &covers, false, None),
            rail,
            self.order_lanes(lanes, &covers, false, None),
            column![above, body],
        ]
        .height(room + tall);
        track::seen(
            track::Key::Whole(id),
            places,
            self.ringed(
                crate::rail::RingOn::All(id),
                self.draggable(id, header.into()),
            ),
        )
    }

    /// The place of a section with nothing in it, under its header:
    /// where its steps will be, dashed in its `color`. A press on it picks
    /// the section, and what is carried over it lands in it. Picked with
    /// others, as the last of them, it has what can be done to them.
    fn empty_place(
        &self,
        section: StepId,
        color: iced::Color,
        picked_end: bool,
        lanes: Lanes<'_>,
        places: &track::Places,
    ) -> Element<'_, Message> {
        let inside = row![
            look::icon(Icon::Plus, 14.0, color),
            text("Drop steps here").size(BODY).font(MEDIUM).color(color),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .padding([0, 12])
        .height(lanes.rows.place() - 8.0);
        let slot = button(crate::drawn::dashed(
            container(inside).width(Fill),
            Palette::tint(color, 0.55),
            12.0,
        ))
        .padding(0)
        .width(Fill)
        .on_press(Message::Click(Clicked::Step(section)))
        .style(|_, _| button::Style::default());
        let mut place = column![container(slot).padding([4, 0])];
        if picked_end {
            place = place.push(self.picked_actions());
        }
        let place = row![
            Space::new().width(lanes.left() + RAIL + lanes.drawn),
            container(place).padding([0, 10]),
        ];
        // Wherever the pointer is on it, what is carried lands in it.
        let area = mouse_area(place)
            .on_enter(Message::Hover(Some((section, Half::Lower))))
            .on_exit(Message::Unhover(section));
        let area = match self.drag.as_ref().is_some_and(|drag| drag.moving) {
            true => area.interaction(mouse::Interaction::Grabbing),
            false => area,
        };
        track::seen(track::Key::Place(section), places, area)
    }

    /// Whether a section is being carried.
    pub(crate) fn carrying_section(&self) -> bool {
        self.drag
            .as_ref()
            .is_some_and(|drag| drag.moving && drag.is_section())
    }

    /// A section over in a service, by the clock of its run.
    pub(crate) fn over(&self, timing: &Timing, id: StepId) -> bool {
        self.mode == crate::mode::Mode::Run
            && timing
                .sections
                .iter()
                .find(|section| section.id == id)
                .is_some_and(|section| matches!(section.phase, Phase::Done { .. }))
    }

    /// A section showing as its one row: folded by hand, or over and not
    /// opened again.
    pub(crate) fn shut(&self, timing: &Timing, id: StepId) -> bool {
        match self.over(timing, id) {
            true => !self.opened.contains(&id),
            false => self.folded.contains(&id),
        }
    }

    /// What a section's ring shows under it: its name (a field while it
    /// is renamed), the time it is given, the time of day it should be
    /// over by, what it takes, and when it is expected to end. In `p`,
    /// the ring's colours, bright over the plan gone dim.
    pub(crate) fn section_card(
        &self,
        p: Palette,
        plan: &Plan,
        step: &Step,
        timing: Option<&SectionTiming>,
    ) -> Element<'_, Message> {
        let id = step.id;
        let budget = match step.action {
            Action::Section { budget, .. } => budget.filter(|budget| !budget.is_zero()),
            _ => None,
        };
        let name: Element<'_, Message> = match self.editing(id, Part::Name) {
            true => self.name_field(step),
            false => container(
                single_line(step.label.clone())
                    .size(BODY)
                    .font(BOLD)
                    .color(p.primary),
            )
            .width(Fill)
            .clip(true)
            .into(),
        };
        let can = self.can_edit();
        let quiet = p.on_surface_variant;
        let small = |label: &'static str, does: Message| -> Element<'_, Message> {
            button(text(label).size(META).font(BOLD).color(p.on_surface))
                .padding([1, 4])
                .on_press_maybe(can.then_some(does))
                .style(look::pill(p, Tone::Plain))
                .into()
        };
        // The time it is given: a minute less or more each press, or
        // typed.
        let goal_words = match budget {
            Some(budget) => length::clock(budget),
            None => "—".to_string(),
        };
        let goal: Element<'_, Message> = match self.editing(id, Part::Length) {
            true => self.section_time_field(id, budget),
            false => button(text(goal_words).size(META).font(BOLD).color(p.on_surface))
                .padding([2, 6])
                .on_press_maybe(can.then_some(Message::EditStep(Some((id, Part::Length)))))
                .style(look::pill(p, Tone::Plain))
                .into(),
        };
        let goal = row![
            look::icon(Icon::Target, 12.0, quiet),
            small("−", Message::SectionGoal(id, -60)),
            goal,
            small("+", Message::SectionGoal(id, 60)),
        ]
        .spacing(3)
        .align_y(Alignment::Center);
        // The time of day it should be over by, set or typed.
        let done_by = step.done_by();
        let by: Element<'_, Message> = match self.editing(id, Part::DoneBy) {
            true => text_input(
                "11:30",
                self.done_typed
                    .as_ref()
                    .filter(|(of, _)| *of == id)
                    .map_or("", |(_, typed)| typed.as_str()),
            )
            .id(crate::plan_edit::part_id(id, Part::DoneBy))
            .on_input(move |typed| Message::DoneBy(id, typed))
            .on_submit(Message::EditStep(None))
            .size(META)
            .width(62)
            .padding([2, 6])
            .into(),
            false => button(
                text(done_by.map_or("—".to_string(), |time| time.to_string()))
                    .size(META)
                    .font(BOLD)
                    .color(p.on_surface),
            )
            .padding([2, 8])
            .on_press_maybe(can.then_some(Message::EditStep(Some((id, Part::DoneBy)))))
            .style(look::pill(p, Tone::Plain))
            .into(),
        };
        let by = row![look::icon(Icon::Clock, 13.0, quiet), by]
            .spacing(4)
            .align_y(Alignment::Center);
        let took = match timing.map(|timing| timing.phase) {
            Some(Phase::Running { elapsed, .. }) => elapsed,
            Some(Phase::Done { took }) => took.unwrap_or_default(),
            Some(Phase::Coming { planned }) => planned,
            None => self.planned(plan, id),
        };
        let over = budget.is_some_and(|budget| took > budget);
        let mut sum = row![
            text(format!("Σ {}", length::clock(took)))
                .size(META)
                .font(BOLD)
                .color(if over { p.error } else { p.on_surface })
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        // How far from the time it is given: over in the colour of what is
        // wrong, under quietly.
        if let Some(budget) = budget.filter(|budget| *budget != took) {
            let (sign, by, ink) = match took > budget {
                true => ("+", took - budget, p.error),
                false => ("−", budget - took, quiet),
            };
            sum = sum.push(
                text(format!("{sign}{}", length::clock(by)))
                    .size(META)
                    .color(ink),
            );
        }
        // When it is expected to end, and how far past its time of day
        // that is.
        let ends: Option<Element<'_, Message>> = timing.and_then(|timing| {
            let ends = timing.ends?;
            Some(match timing.late() {
                Some(late) => text(format!(
                    "ends {ends}, {} min late",
                    late.as_secs().div_ceil(60)
                ))
                .size(META)
                .color(p.error)
                .into(),
                None => text(format!("ends {ends}")).size(META).color(quiet).into(),
            })
        });
        let mut card = column![name, goal, by, sum].spacing(6);
        if let Some(ends) = ends {
            card = card.push(ends);
        }
        card.into()
    }

    /// The room left of the dots in one row, a part for each lane, the
    /// outermost first: where a span that comes again and `covers` this
    /// row runs down a lane, a right click there opens how often it
    /// comes. Not while a dot is pulled (`quiet`).
    fn loop_lanes<'a>(
        &self,
        lanes: Lanes<'_>,
        covers: &dyn Fn(&Laid) -> bool,
        quiet: bool,
        beside: Option<StepId>,
    ) -> Element<'a, Message> {
        let rings = self.can_edit() && !quiet;
        let parts = (0..lanes.loop_lanes).rev().map(|lane| {
            // Each lane is as wide as from one line to the next, about
            // its own line; the outermost takes what room is left over.
            let wide = match lane + 1 == lanes.loop_lanes {
                true => lanes.curl - track::LOOP_STEP * f32::from(lanes.loop_lanes - 1),
                false => track::LOOP_STEP,
            };
            let part = Space::new().width(wide).height(Fill);
            match lanes
                .loops
                .iter()
                .find(|laid| laid.lane == lane && covers(laid))
            {
                Some(laid) if rings => {
                    let line = mouse_area(part)
                        .on_right_press(Message::Rail(crate::rail::Act::Ring(
                            crate::rail::RingOn::Loop(laid.of),
                        )))
                        .on_double_click(Message::Rail(crate::rail::Act::Unloop(laid.of)))
                        .interaction(mouse::Interaction::Pointer);
                    // Beside a step, pressed and pulled along, that part
                    // of the loop is rubbed out; let go where it was
                    // pressed, the steps it holds are picked.
                    match (laid.of, beside) {
                        (FlowOf::Span(span), Some(step))
                            if self.mode == crate::mode::Mode::Plan =>
                        {
                            line.on_press(Message::Rail(crate::rail::Act::PressLoop(step, span)))
                                .into()
                        }
                        _ => line.into(),
                    }
                }
                _ => part.into(),
            }
        });
        match lanes.loop_lanes {
            0 => Space::new().width(lanes.curl).height(Fill).into(),
            _ => row(parts).width(lanes.curl).height(Fill).into(),
        }
    }

    /// The same to the right of the dots, the innermost lane first, for
    /// the lines of a drawn order. Beside a step (`beside`), a line
    /// pressed and pulled along draws more of the order or rubs some of
    /// it out, and a double click takes all of it away.
    fn order_lanes<'a>(
        &self,
        lanes: Lanes<'_>,
        covers: &dyn Fn(&Laid) -> bool,
        quiet: bool,
        beside: Option<StepId>,
    ) -> Element<'a, Message> {
        let pulls = self.mode == crate::mode::Mode::Plan;
        let rings = self.can_edit() && !quiet;
        let parts = (0..lanes.order_lanes).map(|lane| {
            let wide = match lane + 1 == lanes.order_lanes {
                true => lanes.drawn - track::ORDER_STEP * f32::from(lanes.order_lanes - 1),
                false => track::ORDER_STEP,
            };
            let part = Space::new().width(wide).height(Fill);
            match lanes
                .orders
                .iter()
                .find(|laid| laid.lane == lane && covers(laid))
            {
                Some(laid) if rings => {
                    let line = mouse_area(part)
                        .on_right_press(Message::Rail(crate::rail::Act::Ring(
                            crate::rail::RingOn::Shuffle(laid.of),
                        )))
                        .on_double_click(Message::Rail(crate::rail::Act::Unshuffle(laid.of)))
                        .interaction(mouse::Interaction::Pointer);
                    match beside.filter(|_| pulls) {
                        Some(step) => line
                            .on_press(Message::Rail(crate::rail::Act::PressOrder(
                                step,
                                match laid.of {
                                    FlowOf::Span(span) => Some(span),
                                    _ => None,
                                },
                            )))
                            .into(),
                        None => line.into(),
                    }
                }
                _ => part.into(),
            }
        });
        match lanes.order_lanes {
            0 => Space::new().width(lanes.drawn).height(Fill).into(),
            _ => row(parts).width(lanes.drawn).height(Fill).into(),
        }
    }

    /// Whether the pointer is on the row of `id` and the row can change:
    /// not the selected one, which shows what can be done to it already,
    /// and not while something is held or typed.
    fn pointed(&self, id: StepId) -> bool {
        self.hovered.is_some_and(|(over, _)| over == id)
            && self.selected != Some(id)
            && !self.drag.as_ref().is_some_and(|drag| drag.moving)
            && !self.carried.as_ref().is_some_and(|held| held.moving)
            && self.editing.is_none()
            && self.can_edit()
            && (self.mode != crate::mode::Mode::Run || self.ahead(&self.engine.0.plan(), id))
    }

    /// What the row under the pointer offers: Replace, when something
    /// else is picked, and ✕ to take the step out.
    fn pointed_actions(&self, id: StepId) -> Element<'_, Message> {
        let p = self.palette;
        let tip = |about: String| {
            container(text(about).size(META))
                .padding([4, 8])
                .style(look::tip(p))
        };
        // No taller than the name beside them: a row that grew under the
        // pointer pushed every row below it down.
        let mut actions = row![].spacing(2).align_y(Alignment::Center);
        if let Some((_, pick)) = self.replaceable_at(id) {
            actions = actions.push(
                tooltip(
                    button(text("Replace").size(META).font(BOLD).color(p.primary))
                        .padding([0, 8])
                        .on_press(Message::ReplaceStep(id))
                        .style(look::pill(p, Tone::Selected)),
                    tip(format!("Put “{}” here instead", pick.title)),
                    tooltip::Position::Top,
                )
                .delay(look::TIP_DELAY),
            );
        }
        actions
            .push(
                tooltip(
                    button(text("✕").size(META).color(p.on_surface_variant))
                        .padding([0, 6])
                        .on_press(Message::Remove(id))
                        .style(look::row(p, Tone::Plain)),
                    tip(keys::with_key("Remove", Op::RemoveStep)),
                    tooltip::Position::Top,
                )
                .delay(look::TIP_DELAY),
            )
            .into()
    }

    /// Something on standby: its picture and name. A click lines it up,
    /// a second (or Enter) shows it; ✕ takes it off the shelf.
    fn standby_row(&self, resource: &Resource, lined_up: bool) -> Element<'_, Message> {
        let p = self.palette;
        let on_stage = self
            .stage
            .program
            .as_ref()
            .is_some_and(|scene| scene.resource.id == resource.id);
        let tone = match (on_stage, lined_up) {
            (true, _) => Tone::Live,
            (false, true) => Tone::Selected,
            (false, false) => Tone::Plain,
        };
        let color = match tone {
            Tone::Plain => p.on_surface,
            tone => tone.text(p),
        };
        let label = single_line(resource.title.clone())
            .size(BODY)
            .font(if tone == Tone::Plain { MEDIUM } else { BOLD })
            .color(color);
        let off = tooltip(
            button(text("✕").size(META).color(p.on_surface_variant))
                .padding([2, 6])
                .on_press(Message::Unstandby(resource.id.clone()))
                .style(look::row(p, Tone::Plain)),
            container(text("Take it off standby").size(META))
                .padding([4, 8])
                .style(look::tip(p)),
            tooltip::Position::Top,
        )
        .delay(look::TIP_DELAY);
        let line = row![
            Space::new().width(16),
            container(label).width(Fill).clip(true),
            off,
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        button(line)
            .width(Fill)
            .padding([10, 10])
            .on_press(Message::Click(Clicked::Standby(resource.id.clone())))
            .style(look::row(p, tone))
            .into()
    }

    /// How long a step takes: `4:00` when known, `~4 min` when guessed,
    /// nothing for a step that shows nothing.
    pub(crate) fn step_length(&self, step: &Step) -> String {
        length_words(lantern_engine::timing::estimate(
            step,
            self.loaded_length(step),
        ))
    }

    /// What is known of what `step` shows, for its row while planning.
    pub(crate) fn known(&self, step: &Step) -> Known {
        match &step.action {
            Action::Show { resource } => self.known_of(resource),
            _ => Known::default(),
        }
    }

    /// What is known of `resource`: how long it plays, how many pages
    /// it has, how far its download has come, how big it is and the
    /// screen it is shown on.
    pub(crate) fn known_of(&self, resource: &Resource) -> Known {
        Known {
            length: Some(lantern_engine::timing::media_length(
                resource,
                self.resource_length(resource),
            )),
            pages: self.media.pages(&resource.id),
            download: self.download_part(resource),
            pixels: self.thumbs.pixels(&resource.id),
            screen: self.stage_screen,
            reach: crate::ready::reach(
                resource,
                |video| self.downloads.0.state(&video.id),
                crate::stage::image_cached,
            ),
        }
    }

    /// What a search result's row says of `resource`: its picture's size
    /// and state, and where it's from or the one warning that matters.
    pub(crate) fn result_facts<'a>(&self, resource: &'a Resource) -> ResultFacts<'a> {
        let on_stage = self
            .stage
            .program
            .as_ref()
            .is_some_and(|scene| scene.resource.id == resource.id);
        let issue = crate::ready::issue_of(
            resource,
            self.config.youtube,
            |video| self.downloads.0.state(&video.id),
            crate::stage::image_cached,
        );
        ResultFacts::of(resource, self.known_of(resource), issue, on_stage)
    }

    /// Where a plan made on another computer is looked for, while that is
    /// open: what was found, and a box to type where it is.
    fn import_rows(&self) -> Option<Element<'_, Message>> {
        let p = self.palette;
        let (typed, found) = self.importing.as_ref()?;
        let mut import = column![].spacing(2);
        for bundle in found {
            let name = bundle
                .file_stem()
                .map_or_else(|| file_name(bundle), |n| n.to_string_lossy().into_owned());
            let place = bundle.parent().map_or(String::new(), |dir| {
                match &self.home {
                    Some(home) => crate::config::tilde(dir, home),
                    None => dir.to_path_buf(),
                }
                .display()
                .to_string()
            });
            import = import.push(
                button(
                    column![
                        single_line(name)
                            .size(BODY)
                            .font(MEDIUM)
                            .color(p.on_surface),
                        single_line(place).size(META).color(p.on_surface_variant),
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding([8, 10])
                .on_press_maybe((!self.copying).then(|| Message::Import(Some(bundle.clone()))))
                .style(look::row(p, Tone::Plain)),
            );
        }
        let hint = if found.is_empty() {
            "No exported plan in Documents, Downloads or on a USB stick. Type where the file is:"
        } else {
            "Or type where the file is:"
        };
        import = import
            .push(container(text(hint).size(META).color(p.on_surface_variant)).padding([4, 10]));
        import = import.push(
            container(
                text_input("e.g. ~/Documents/Sábado.lantern", typed)
                    .id(crate::dashboard::IMPORT_BOX)
                    .on_input(Message::ImportPath)
                    .on_submit(Message::Import(None))
                    .size(BODY)
                    .padding([6, 10])
                    .style(look::input(p)),
            )
            .padding([0, 10]),
        );
        Some(import.into())
    }

    fn saved_plans<'a>(&'a self, entries: &'a [Entry]) -> Element<'a, Message> {
        let p = self.palette;
        // A heading is its name: the rows under it say what they are.
        let heading = |title: &'static str| {
            container(text(title).size(BODY).font(BOLD).color(p.primary)).padding([4, 8])
        };
        let is_template = |entry: &Entry| entry.summary.as_ref().is_ok_and(|s| s.template);
        // The row the arrows are on, for Enter.
        let picked = |entry: &Entry| self.list_pick.as_deref() == Some(entry.path.as_path());
        let now = SystemTime::now();
        let when = |entry: &Entry| ago(now.duration_since(entry.modified).unwrap_or_default());
        let mut list = column![].spacing(2);
        // The plan that's open, first: the way back to it.
        let open = self.engine.0.plan();
        if self.plans.has_plan(&open) {
            let steps = open.steps().iter().filter(|s| !s.is_section()).count();
            let detail = if self.plans.is_dirty(&open) {
                format!("{} · unsaved changes", count(steps, "step"))
            } else {
                count(steps, "step")
            };
            list = list.push(heading("Open now"));
            list = list.push(
                button(
                    column![
                        single_line(open.title.clone())
                            .size(BODY)
                            .font(BOLD)
                            .color(p.primary),
                        single_line(detail).size(META).color(p.on_surface_variant),
                    ]
                    .spacing(2),
                )
                .width(Fill)
                .padding([8, 10])
                .on_press(Message::ClosePlanMode)
                .style(look::row(p, Tone::Selected)),
            );
            list = list.push(Space::new().height(10));
        }
        let templates: Vec<&Entry> = entries.iter().filter(|e| is_template(e)).collect();
        if !templates.is_empty() {
            list = list.push(heading("Templates"));
            for entry in templates {
                list = list.push(saved_plan(
                    p,
                    entry,
                    self.listed_detail(entry, when(entry)),
                    picked(entry),
                ));
            }
            list = list.push(Space::new().height(10));
        }
        // Where the files are is in the README, for whoever goes looking.
        // The open plan is already at the top, under "Open now".
        let plans: Vec<&Entry> = entries
            .iter()
            .filter(|e| !is_template(e) && Some(e.path.as_path()) != self.plans.file())
            .collect();
        list = list.push(heading("Your plans"));
        list = list.push(hinted(
            p,
            list_action(p, "+", "New plan".to_string(), op(Op::New)),
            keys::with_key("Start an empty plan", Op::New),
            tooltip::Position::Bottom,
        ));
        for entry in plans {
            list = list.push(saved_plan(
                p,
                entry,
                self.listed_detail(entry, when(entry)),
                picked(entry),
            ));
        }
        // A plan made on another computer, brought here with its media.
        list = list.push(Space::new().height(10));
        list = list.push(list_action(
            p,
            if self.importing.is_some() {
                "−"
            } else {
                "↓"
            },
            "Import a plan".into(),
            Message::ToggleImport,
        ));
        if let Some(import) = self.import_rows() {
            list = list.push(container(import).padding(iced::Padding::default().left(24.0)));
        }
        scrolling(list).into()
    }
}

/// A row that does something to its list, at its end or its head: "+ New
/// plan", "+ Add step".
fn list_action<'a>(
    p: Palette,
    sign: &'a str,
    label: String,
    message: Message,
) -> Element<'a, Message> {
    breathing_action(p, sign, label, message, 0.0)
}

/// A row's action, its sign `breath` of the way to its fullest: larger,
/// and in the accent colour.
fn breathing_action<'a>(
    p: Palette,
    sign: &'a str,
    label: String,
    message: Message,
    breath: f32,
) -> Element<'a, Message> {
    button(
        row![
            text(sign)
                .size(HEADING + 8.0 * breath)
                .line_height(iced::widget::text::LineHeight::Absolute(HEADING.into()))
                .color(p.on_surface_variant)
                .width(16)
                .align_x(iced::alignment::Horizontal::Center),
            text(label).size(BODY).color(p.on_surface_variant),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding([8, 10])
    .on_press(message)
    .style(look::row(p, Tone::Plain))
    .into()
}

/// The running section, for the launcher's header: "Alabanza · 8:20 left",
/// "Alabanza · 2:10 over".
pub(crate) fn section_status(section: &SectionTiming) -> (String, Tone) {
    let Phase::Running { elapsed, expected } = section.phase else {
        return (section.title.clone(), Tone::Plain);
    };
    let Some(budget) = section.budget else {
        return (
            format!("{} · {}", section.title, length::clock(elapsed)),
            Tone::Plain,
        );
    };
    let pace = Pace::of(expected, budget);
    let text = match pace {
        Pace::Over => format!(
            "{} · {} over",
            section.title,
            length::clock(expected.saturating_sub(budget))
        ),
        Pace::OnTrack | Pace::Tight => format!(
            "{} · {} left",
            section.title,
            length::clock(budget.saturating_sub(elapsed))
        ),
    };
    (text, pace_tone(pace))
}

/// What the top bar holds the service against: the innermost section
/// under way that has a goal, else the plan's.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Goal {
    pub(crate) name: String,
    /// The section it is, where it is one and not the plan's.
    pub(crate) section: Option<StepId>,
    pub(crate) so_far: Duration,
    pub(crate) goal: Duration,
    pub(crate) tone: Tone,
}

/// The colour of the section `id`, by what [`Plan::section_colors`]
/// gave it.
pub(crate) fn section_hue(p: Palette, colors: &[(StepId, u8)], id: StepId) -> iced::Color {
    p.section(
        colors
            .iter()
            .find(|(section, _)| *section == id)
            .map_or(0, |(_, slot)| *slot),
    )
}

/// The goal of where the service is. `plan` is the plan's name, its
/// goal, how long it has run, and how long it is expected to take.
pub(crate) fn goal_of(
    timing: &Timing,
    plan: Option<(&str, Duration, Duration, Duration)>,
) -> Option<Goal> {
    // Sections come outermost first: the last under way is the
    // innermost.
    let section = timing
        .sections
        .iter()
        .rev()
        .find_map(|section| match section.phase {
            Phase::Running { elapsed, expected } => {
                let goal = section.budget.filter(|goal| !goal.is_zero())?;
                Some(Goal {
                    name: section.title.clone(),
                    section: Some(section.id),
                    so_far: elapsed,
                    goal,
                    tone: pace_tone(Pace::of(expected, goal)),
                })
            }
            _ => None,
        });
    section.or_else(|| {
        let (name, goal, so_far, expected) = plan.filter(|(_, goal, ..)| !goal.is_zero())?;
        Some(Goal {
            name: name.to_string(),
            section: None,
            so_far,
            goal,
            tone: pace_tone(Pace::of(expected, goal)),
        })
    })
}

/// How long `step` fades in and out, each a second less or more with −
/// and +; a chooser among them is `width` wide.
pub(crate) fn fade_steppers<'a>(p: Palette, step: &Step, width: Length) -> Element<'a, Message> {
    let id = step.id;
    // What it plays apart fades with what it shows, or on its own; the
    // two say which is which as the lines above do.
    let apart = crate::sources::plays_apart(step);
    let mut steppers = fade_pair(
        p,
        apart.then_some(Icon::Image),
        step.fade,
        ["Fades in", "Fades out"],
        move |end, by| Message::SetFade(id, end, by),
    );
    if apart {
        steppers = steppers.push(crate::sources::sound_fade_chooser(p, step, width));
        if let Some(fade) = step.sound_fade {
            steppers = steppers.push(fade_pair(
                p,
                Some(Icon::Music),
                fade,
                ["Its sound fades in", "Its sound fades out"],
                move |end, by| Message::SoundFade(id, Some((end, by))),
            ));
        }
    }
    steppers.into()
}

/// A fade's two ends, each with its ramp, its time, and − and + that
/// make it shorter or longer.
fn fade_pair<'a>(
    p: Palette,
    mark: Option<Icon>,
    fade: lantern_engine::Fade,
    names: [&'static str; 2],
    set: impl Fn(FadeEnd, i64) -> Message + Copy + 'a,
) -> iced::widget::Column<'a, Message> {
    let small = move |label: &'static str, does: Option<Message>| {
        button(text(label).size(META).font(BOLD).color(p.on_surface))
            .padding([1, 4])
            .on_press_maybe(does)
            .style(look::pill(p, Tone::Plain))
    };
    // Its ramp says which end it is, as on the row: up in, down out.
    let one = move |name: &'static str, end: FadeEnd, now: Option<FadeTime>| {
        let shown = now.map_or("—".to_string(), |time| format!("{} s", time.secs()));
        let ramp = look::ramp(end == FadeEnd::In, end == FadeEnd::Out, 10.0, p.primary);
        row![
            mark.map(|mark| look::icon(mark, 12.0, p.on_surface_variant)),
            hinted(p, ramp, name.to_string(), tooltip::Position::Left),
            small("−", now.map(|_| set(end, -1))),
            container(text(shown).size(META).font(BOLD).color(p.on_surface)).center_x(28),
            small("+", (now != Some(FadeTime::LONGEST)).then(|| set(end, 1))),
        ]
        .spacing(3)
        .align_y(Alignment::Center)
    };
    column![
        one(names[0], FadeEnd::In, fade.fade_in),
        one(names[1], FadeEnd::Out, fade.fade_out),
    ]
    .spacing(4)
}

/// `words` at a row's end, after a small ramp where the step fades: up
/// where it fades in, down where it fades out.
fn ramped<'a>(
    step: &Step,
    words: Element<'a, Message>,
    color: iced::Color,
) -> Element<'a, Message> {
    if !matches!(step.action, Action::Show { .. }) || step.fade.is_none() {
        return words;
    }
    let (fades_in, fades_out) = (step.fade.fade_in.is_some(), step.fade.fade_out.is_some());
    row![look::ramp(fades_in, fades_out, 8.0, color), words]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
}

/// The line under a step's name while planning: what it is, quietly, or
/// what is wrong with it in the colour of how bad that is.
pub(crate) fn under_name<'a>(p: Palette, under: &Under) -> Element<'a, Message> {
    let (words, ink) = match under {
        Under::What(words) => (words.clone(), p.on_surface_variant),
        Under::Wrong(level, words) => (words.clone(), level_tone(Some(*level)).text(p)),
        Under::Soft(screen) => (soft_words(*screen), p.tertiary),
    };
    single_line(words).size(META).color(ink).into()
}

/// What a row says of a picture the stage must scale up to fit `screen`.
pub(crate) fn soft_words(screen: Pixels) -> String {
    format!("Smaller than the screen ({screen}): it will look soft")
}

/// The mark beside a step's length while planning: whether it goes on
/// by itself when it is done, in the colour of the line on to the next,
/// or waits. Pointed at, it says which.
fn goes_on_mark<'a>(p: Palette, goes_on: bool) -> Element<'a, Message> {
    let (icon, ink, says) = match goes_on {
        true => (Icon::Joined, p.primary, "Goes straight on by itself"),
        false => (Icon::Pause, p.on_surface_variant, "Waits for you"),
    };
    hinted(
        p,
        look::icon(icon, 13.0, ink),
        says.to_string(),
        tooltip::Position::Top,
    )
}

/// How far into its media the stage is: "slide 2 of 5", "page 1 of 3",
/// "1:22 / 4:26", and the share done. `None` for what has no progress (a
/// still image, a one-slide text, a video of unknown length).
pub(crate) fn progress(media: &Media, resource: &Resource) -> Option<(String, f32)> {
    match media {
        Media::Frames {
            frame,
            count: Some(count),
        } if count.get() > 1 => Some((
            format!("{} {} of {count}", unit(resource), frame + 1),
            (frame + 1) as f32 / count.get() as f32,
        )),
        Media::Timed(timed) => {
            let duration = timed.duration.filter(|d| !d.is_zero())?;
            Some((
                format!(
                    "{} / {}",
                    length::clock(timed.position),
                    length::clock(duration)
                ),
                (timed.position.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0),
            ))
        }
        _ => None,
    }
}

/// A thin bar, `done` of it filled in `color`, `indent` in from the left.
pub(crate) fn bar<'a>(
    p: Palette,
    done: f32,
    color: iced::Color,
    indent: f32,
) -> Element<'a, Message> {
    container(widgets::bar(
        [
            (done, Filled::Solid(color)),
            (1.0 - done, Filled::Solid(p.surface_container_highest)),
        ],
        3.0,
    ))
    .padding(iced::Padding::default().left(indent))
    .into()
}

/// A live slot's line: how much was shown in it, then what it searches,
/// "2 shown · !yt" (the count first: it's what changes, and a long search
/// may be cut at the row's edge).
pub(crate) fn slot_detail(hint: Option<&Query>, shown: usize) -> String {
    let mut detail = vec![match shown {
        0 => "live search".to_string(),
        n => format!("{n} shown"),
    }];
    detail.extend(
        hint.map(|q| q.text.trim().to_string())
            .filter(|t| !t.is_empty()),
    );
    detail.join(" · ")
}

/// On track is quiet; tight is the warning tint, over the error tint.
fn pace_tone(pace: Pace) -> Tone {
    match pace {
        Pace::OnTrack => Tone::Plain,
        Pace::Tight => Tone::Live,
        Pace::Over => Tone::Danger,
    }
}

/// A saved plan in the list. Clicking a plan opens it; clicking a template
/// starts a new plan from it, and its Edit opens the template itself. The
/// star makes a plan a template, or not.
fn saved_plan(p: Palette, entry: &Entry, detail: String, picked: bool) -> Element<'_, Message> {
    let (title, detail, color, template) = match &entry.summary {
        Ok(summary) => (
            summary.title.clone(),
            detail,
            p.on_surface,
            summary.template,
        ),
        Err(why) => (
            file_name(&entry.path),
            format!("can't read: {why}"),
            p.error,
            false,
        ),
    };
    let readable = entry.summary.is_ok();
    let path = entry.path.clone();
    let main = button(
        column![
            single_line(title).size(BODY).font(MEDIUM).color(color),
            single_line(detail).size(META).color(p.on_surface_variant),
        ]
        .spacing(2),
    )
    .width(Fill)
    .padding([8, 10])
    .on_press_maybe(readable.then(|| {
        if template {
            Message::FromTemplate(path.clone())
        } else {
            Message::OpenPlan(path.clone())
        }
    }))
    .style(look::row(
        p,
        if picked { Tone::Selected } else { Tone::Plain },
    ));
    let star = tooltip(
        button(
            text(if template { "★" } else { "☆" })
                .size(BODY)
                .color(p.primary),
        )
        .padding([6, 8])
        .on_press_maybe(readable.then(|| Message::SetTemplate(path.clone(), !template)))
        .style(look::row(p, Tone::Plain)),
        container(
            text(if template {
                "Make it a plan again"
            } else {
                "Make it a template"
            })
            .size(META)
            .color(p.on_surface),
        )
        .padding([4, 8])
        .style(look::tip(p)),
        // Above the star: to its left it covered the Edit button.
        tooltip::Position::Top,
    )
    .delay(look::TIP_DELAY);
    let mut line = row![main].spacing(4).align_y(Alignment::Center);
    if template {
        line = line.push(pill(
            p,
            "Edit",
            Tone::Plain,
            PILL_SMALL,
            Some(Message::OpenPlan(entry.path.clone())),
        ));
    }
    line.push(star).into()
}

/// Asks before `switch` replaces the plan: `leaving` a plan that's
/// running, or else discarding one that was never saved.
fn confirm<'a>(p: Palette, plan: &Plan, switch: &Switch, leaving: bool) -> Element<'a, Message> {
    let then = match switch {
        Switch::New => "start a new one",
        Switch::Open(_) => "open another",
        Switch::FromTemplate(_) => "start one from the template",
    };
    let (ask, more, yes, go, no) = if leaving {
        (
            format!("“{}” is running. Leave it and {then}?", plan.title),
            "What's on stage stays there. The plan is in Plans to come back to.".to_string(),
            "Leave it",
            Message::LeavePlan,
            "Stay",
        )
    } else {
        (
            format!("“{}” was never saved. Discard it and {then}?", plan.title),
            format!("{} saves it instead.", keys::label(Op::Save)),
            "Discard",
            Message::DiscardPlan,
            "Keep it",
        )
    };
    column![
        text(ask).size(BODY),
        text(more).size(META).color(p.on_surface_variant),
        row![
            wide(pill(p, yes, Tone::Danger, PILL_SMALL, Some(go))),
            wide(pill(
                p,
                no,
                Tone::Plain,
                PILL_SMALL,
                Some(Message::ClosePlanMode)
            )),
        ]
        .spacing(4),
    ]
    .spacing(10)
    .padding([4, 8])
    .height(Fill)
    .into()
}

/// "1 step", "3 steps".
pub(crate) fn count(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// How long ago, roughly: "just now", "5 min ago", "3 h ago", "2 days ago".
fn ago(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes} min ago"),
        60..1440 => format!("{} h ago", minutes / 60),
        1440..2880 => "1 day ago".into(),
        _ => format!("{} days ago", minutes / 1440),
    }
}

/// How often something comes, in a few characters; `None` for once.
pub(crate) fn repeat_words(repeat: Repeat) -> Option<String> {
    match repeat {
        Repeat::Once => None,
        Repeat::Times(times) => Some(format!("×{times}")),
        Repeat::Forever => Some("∞".into()),
        Repeat::Until(time) => Some(time.to_string()),
    }
}

/// How tall the marks under a section's gap are, with their space.
const RULER_TALL: f32 = 14.0;

/// The levels a carried section could take where it would land, a mark
/// each: the one it has is filled, the others it could have are rings,
/// and one out of reach there is a speck.
fn level_ruler<'a>(p: Palette, at: u8, least: u8, most: u8) -> Element<'a, Message> {
    let mark = |level: u8| -> Element<'a, Message> {
        let (size, fill, edge) = match level {
            level if level == at => (10.0, Some(p.primary), None),
            level if (least..=most).contains(&level) => (10.0, None, Some(p.on_surface_variant)),
            _ => (4.0, Some(p.outline_variant), None),
        };
        container(
            container(Space::new().width(size).height(size)).style(move |_| container::Style {
                background: fill.map(Into::into),
                border: iced::Border {
                    color: edge.unwrap_or(iced::Color::TRANSPARENT),
                    width: if edge.is_some() { 2.0 } else { 0.0 },
                    radius: (size / 2.0).into(),
                },
                ..container::Style::default()
            }),
        )
        .center_x(10)
        .center_y(10)
        .into()
    };
    let levels = f32::from(lantern_engine::DEEPEST) + 1.0;
    // A thin line runs behind them, from the first to the last.
    let line = container(
        container(Space::new().width(Fill).height(2))
            .style(move |_| container::Style::default().background(p.outline_variant)),
    )
    .width(levels * 10.0 + (levels - 1.0) * 6.0)
    .center_y(10)
    .padding([0, 5]);
    let marks = row((0..=lantern_engine::DEEPEST).map(mark)).spacing(6);
    container(stack![line, marks])
        .padding(iced::Padding::default().left(8.0))
        .into()
}

/// Where a drag would land: a highlighted slot naming what it carries.
fn drop_gap<'a>(p: Palette, label: String) -> Element<'a, Message> {
    // On one line whatever its length: the gap is as tall for every
    // row, so one closing and one opening add up to the same height.
    container(single_line(label).size(BODY).font(MEDIUM).color(p.primary))
        .width(Fill)
        .clip(true)
        .padding([12, 12])
        .style(move |_| container::Style {
            background: Some(Palette::tint(p.primary, 0.12).into()),
            border: iced::Border {
                color: p.primary,
                width: 2.0,
                radius: 14.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// "12 s", "1:05": a countdown, rounded up so it never shows 0 early.
fn seconds_left(left: Duration) -> String {
    let secs = left.as_secs() + u64::from(left.subsec_nanos() > 0);
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::StepId;

    use super::*;

    const MIN: Duration = Duration::from_secs(60);
    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn the_title_says_a_plans_steps_its_sum_its_goal_and_its_times() {
        let sums = |filled: Duration, total: Duration, open: bool| PlanSums {
            filled,
            total,
            open,
        };
        let fact = |words: &str, over: bool| (words.to_string(), over);
        // Nothing set: how many steps, and what they come to.
        assert_eq!(
            plan_facts(5, None, None, &sums(40 * MIN, 40 * MIN, false)),
            [fact("5 steps", false), fact("Σ 40:00", false)]
        );
        // Something comes again with no end: at least that.
        assert_eq!(
            plan_facts(1, None, None, &sums(40 * MIN, 40 * MIN, true)),
            [fact("1 step", false), fact("Σ 40:00+", false)]
        );
        // Over its goal, the sum says so; it ends after its goal.
        let starts = TimeOfDay::parse("19:00");
        assert_eq!(
            plan_facts(3, Some(45 * MIN), starts, &sums(50 * MIN, 50 * MIN, false)),
            [
                fact("3 steps", false),
                fact("Σ 50:00", true),
                fact("◎ 45:00", false),
                fact("starts 19:00, ends 19:45", false),
            ]
        );
        assert!(!plan_facts(3, Some(45 * MIN), None, &sums(45 * MIN, 45 * MIN, false))[1].1);
        // Without a goal it is held against its sections' times, which are
        // no goal to be over, and ends after them.
        assert_eq!(
            plan_facts(3, None, starts, &sums(5 * MIN, 35 * MIN, false)),
            [
                fact("3 steps", false),
                fact("Σ 5:00", false),
                fact("◎ 35:00", false),
                fact("starts 19:00, ends 19:35", false),
            ]
        );
    }

    #[test]
    fn a_plan_is_held_against_its_goal_or_else_its_sections_times() {
        let sums = |filled: Duration, total: Duration| PlanSums {
            filled,
            total,
            open: false,
        };
        assert_eq!(
            sums(20 * MIN, 60 * MIN).against(Some(45 * MIN)),
            Some(45 * MIN)
        );
        assert_eq!(sums(20 * MIN, 60 * MIN).against(None), Some(60 * MIN));
        assert_eq!(sums(20 * MIN, 20 * MIN).against(None), None);
    }

    #[test]
    fn a_sections_gauge_fills_as_its_steps_take_its_time() {
        assert_eq!(part_fill(Some(10 * MIN), 5 * MIN), (0.5, false));
        assert_eq!(part_fill(Some(10 * MIN), 12 * MIN), (1.0, true));
        assert_eq!(part_fill(None, 5 * MIN), (1.0, false));
    }

    #[test]
    fn a_sections_part_is_over_by_what_its_steps_take_past_its_time() {
        assert_eq!(over_share(Some(10 * MIN), 15 * MIN), 1.0 / 3.0);
        assert_eq!(over_share(Some(10 * MIN), 10 * MIN), 0.0);
        assert_eq!(over_share(Some(10 * MIN), 5 * MIN), 0.0);
        assert_eq!(over_share(None, 5 * MIN), 0.0);
    }

    #[test]
    fn a_sections_part_says_as_much_as_it_has_room_for() {
        let words = ["Bienvenida", "0:24"];
        assert_eq!(words_said(400.0, &words), 2, "its name and its time");
        assert_eq!(words_said(60.0, &words), 1, "its name alone, cut");
        assert_eq!(words_said(20.0, &words), 0, "nothing readable");
        assert_eq!(words_said(60.0, &["empty"]), 1);
    }

    #[test]
    fn ages_are_rough() {
        let minutes = |m: u64| ago(Duration::from_secs(m * 60));
        assert_eq!(minutes(0), "just now");
        assert_eq!(minutes(5), "5 min ago");
        assert_eq!(minutes(130), "2 h ago");
        assert_eq!(minutes(1500), "1 day ago");
        assert_eq!(minutes(3 * 1440), "3 days ago");
    }

    fn section(budget: Option<Duration>, phase: Phase) -> SectionTiming {
        let mut plan = Plan::new("x");
        let id: StepId = plan.section("Alabanza", budget);
        SectionTiming {
            id,
            title: "Alabanza".into(),
            budget,
            phase,
            done_by: None,
            ends: None,
        }
    }

    #[test]
    fn the_goal_is_the_innermost_section_s_else_the_plan_s() {
        let goal = |budget: Option<Duration>, phase: Phase, title: &str| SectionTiming {
            title: title.to_string(),
            budget,
            phase,
            ..section(budget, phase)
        };
        let running = |elapsed: Duration| Phase::Running {
            elapsed,
            expected: elapsed,
        };
        let timing = Timing {
            sections: vec![
                goal(Some(45 * MIN), running(20 * MIN), "Culto"),
                goal(Some(15 * MIN), running(8 * MIN), "Alabanza"),
            ],
            end: None,
        };
        let plan = Some(("Sábado", 180 * MIN, 30 * MIN, 170 * MIN));
        assert_eq!(goal_of(&timing, plan).unwrap().name, "Alabanza");
        let timing = Timing {
            sections: vec![
                goal(Some(45 * MIN), running(20 * MIN), "Culto"),
                goal(None, running(8 * MIN), "Alabanza"),
            ],
            end: None,
        };
        assert_eq!(
            goal_of(&timing, plan).unwrap().name,
            "Culto",
            "the one with a goal"
        );
        let timing = Timing {
            sections: vec![goal(None, running(8 * MIN), "Alabanza")],
            end: None,
        };
        let plan_goal = goal_of(&timing, plan).unwrap();
        assert_eq!(
            (plan_goal.name.as_str(), plan_goal.so_far),
            ("Sábado", 30 * MIN)
        );
        assert_eq!(goal_of(&timing, None), None);
        let over = Timing {
            sections: vec![goal(Some(15 * MIN), running(16 * MIN), "Alabanza")],
            end: None,
        };
        assert_eq!(goal_of(&over, None).unwrap().tone, Tone::Danger);
    }

    #[test]
    fn the_header_says_time_left_or_how_far_over() {
        let running = |elapsed, expected| Phase::Running { elapsed, expected };
        let (text, tone) = section_status(&section(Some(15 * MIN), running(5 * MIN, 10 * MIN)));
        assert_eq!(
            (text.as_str(), tone),
            ("Alabanza · 10:00 left", Tone::Plain)
        );
        let (text, tone) = section_status(&section(Some(15 * MIN), running(14 * MIN, 17 * MIN)));
        assert_eq!(
            (text.as_str(), tone),
            ("Alabanza · 2:00 over", Tone::Danger)
        );
        let (text, _) = section_status(&section(None, running(3 * MIN, 3 * MIN)));
        assert_eq!(text, "Alabanza · 3:00");
    }

    #[test]
    fn a_live_slot_says_what_it_searches_and_what_it_showed() {
        assert_eq!(slot_detail(None, 0), "live search");
        assert_eq!(slot_detail(Some(&Query::new("!yt ")), 2), "2 shown · !yt");
    }

    fn media(kind: Kind, from: Option<&str>) -> Resource {
        let source = lantern_engine::SourceId::new("test").unwrap();
        let resource = Resource::new(
            lantern_engine::ResourceId::new(&source, "x"),
            "x",
            kind,
            lantern_engine::Location::File("/media/x".into()),
        );
        match from {
            Some(from) => resource.with_subtitle(from),
            None => resource,
        }
    }

    #[test]
    fn a_planned_row_says_where_a_thing_is_from_what_it_is_and_its_size() {
        let mut plan = Plan::new("t");
        let picture = plan.show(media(Kind::Image, Some("Fixtures")));
        let video = plan.show(media(Kind::Video, Some("Fixtures")));
        let pdf = plan.show(media(Kind::Document, Some("Fixtures")));
        let page = plan.show(media(Kind::Document, Some("Fixtures")).page(1));
        let typed = plan.show(media(Kind::Text, Some("your text")));
        let bare = plan.show(media(Kind::Text, None));
        let slot = plan.live("Live search", Some(Query::new("!b jn 3")));
        let black = plan.push("Black screen", Action::Blank);
        let facts = |id: StepId, known: Known| {
            RowFacts::of(plan.get(id).unwrap(), None, known, None, false)
        };
        let what = |words: &str| Under::What(words.to_string());
        let lasts = |seconds: u64, basis: Basis| Known {
            length: Some(Estimate {
                length: Duration::from_secs(seconds),
                basis,
            }),
            ..Known::default()
        };
        let pages = |pages: usize| Known {
            pages: Some(pages),
            ..Known::default()
        };
        let full_hd = Pixels {
            width: 1920,
            height: 1080,
        };
        let sharp = Known {
            pixels: Some(full_hd),
            screen: full_hd,
            ..Known::default()
        };

        let still = facts(picture, Known::default());
        assert_eq!(still.under, what("Fixtures · picture"));
        assert_eq!(still.thumb.size, None, "its size isn't known yet");
        let still = facts(picture, sharp);
        assert_eq!(still.thumb.size.as_deref(), Some("1920×1080"));
        assert_eq!(still.thumb.state, None, "as big as the screen");
        assert_eq!(still.under, what("Fixtures · picture"));
        let played = facts(video, lasts(6, Basis::Known));
        assert_eq!(played.under, what("Fixtures · video"));
        assert_eq!(played.thumb.size.as_deref(), Some("0:06"));
        assert_eq!(
            facts(video, lasts(240, Basis::Guess)).thumb.size.as_deref(),
            Some("~4 min"),
            "a guess looks like one"
        );
        assert_eq!(facts(pdf, pages(3)).thumb.size.as_deref(), Some("3 pages"));
        assert_eq!(facts(pdf, Known::default()).thumb.size, None);
        let one = facts(page, pages(3));
        assert_eq!(one.under, what("Fixtures · page"));
        assert_eq!(one.thumb.size, None, "a page is one page");
        assert_eq!(facts(typed, Known::default()).under, what("your text"));
        assert_eq!(facts(bare, Known::default()).under, what("text"));
        assert_eq!(
            facts(slot, Known::default()).under,
            what("live search · !b jn 3")
        );
        let blank = facts(black, Known::default());
        assert_eq!(blank.thumb.picture, Picture::Kind(Icon::ScreenOff));
        assert_eq!(blank.under, what("nothing on the projector"));
    }

    #[test]
    fn the_preview_says_where_a_step_is_from_how_it_plays_its_look_and_its_place() {
        let mut plan = Plan::new("t");
        plan.section("Bienvenida", None);
        plan.show(media(Kind::Image, Some("Fixtures")));
        let praise = plan.section("Alabanza", None);
        let song = plan.show(media(Kind::Video, Some("Fixtures / Kids")));
        plan.push("Black screen", Action::Blank);
        let words = |words: &str| Says::Words(words.to_string());
        let place = |which: &str| Says::Section(praise, "Alabanza".into(), which.into());
        let sized = Known {
            pixels: Some(Pixels {
                width: 1920,
                height: 1080,
            }),
            ..Known::default()
        };
        assert_eq!(
            step_facts(&plan, 1, "~30 s", sized)[..2],
            [
                ("From", words("Fixtures / x")),
                ("Size", words("1920 × 1080")),
            ],
            "a picture's size"
        );
        assert_eq!(
            step_facts(&plan, 3, "0:06", sized),
            [
                ("From", words("Fixtures / Kids / x")),
                ("Plays", words("0:06, then waits")),
                ("Look", words("the plan's")),
                ("Section", place("step 2 of 3")),
            ],
            "a video's size is how long it plays"
        );
        let look = |plan: &Plan| step_facts(plan, 3, "0:06", Known::default()).remove(2);
        plan.get_mut(song).unwrap().fade.fade_in = FadeTime::from_secs(2);
        assert_eq!(look(&plan), ("Look", words("the plan's · fades in 2 s")));
        let change = lantern_engine::LookChange {
            background: Some(lantern_engine::Rgb([1, 2, 3])),
            ..lantern_engine::LookChange::default()
        };
        plan.get_mut(praise).unwrap().look = Some(change.clone());
        assert_eq!(look(&plan), ("Look", words("Alabanza's · fades in 2 s")));
        plan.get_mut(song).unwrap().look = Some(change);
        assert_eq!(look(&plan), ("Look", words("its own · fades in 2 s")));
        // A black screen is from nowhere and has no look of its own.
        plan.flow.joined = true;
        assert_eq!(
            step_facts(&plan, 4, "", Known::default()),
            [
                ("Plays", words("goes on by itself")),
                ("Section", place("step 3 of 3")),
            ]
        );
    }

    #[test]
    fn the_preview_says_where_an_item_is_from_what_it_is_and_its_size() {
        let words = |words: &str| Says::Words(words.to_string());
        let lasts = Known {
            length: Some(Estimate {
                length: Duration::from_secs(6),
                basis: Basis::Known,
            }),
            ..Known::default()
        };
        assert_eq!(
            item_facts(&media(Kind::Video, Some("Fixtures")), lasts, None),
            [
                ("From", words("Fixtures / x")),
                ("What", words("video")),
                ("Length", words("0:06")),
            ]
        );
        assert_eq!(
            item_facts(&media(Kind::Image, None), Known::default(), None),
            [("From", words("x")), ("What", words("picture"))],
            "a picture's size isn't known"
        );
        let sized = Known {
            pixels: Some(Pixels {
                width: 800,
                height: 450,
            }),
            ..Known::default()
        };
        assert_eq!(
            item_facts(&media(Kind::Image, None), sized, None).pop(),
            Some(("Size", words("800 × 450")))
        );
        assert_eq!(
            item_facts(&media(Kind::Document, None), Known::default(), Some(3)).pop(),
            Some(("Size", words("3 pages"))),
            "pages loaded for the preview count"
        );
        let source = lantern_engine::SourceId::new("bible").unwrap();
        let passage = Resource::new(
            lantern_engine::ResourceId::new(&source, "jn3"),
            "Juan 3:16-17",
            Kind::Text,
            Location::Inline("Porque de tal manera".into()),
        )
        .with_subtitle("RV1909");
        assert_eq!(
            item_facts(&passage, Known::default(), Some(2)),
            [
                ("From", words("RV1909 · Juan 3:16-17")),
                ("Size", words("2 slides")),
            ]
        );
    }

    #[test]
    fn a_planned_row_says_its_issue_unless_its_picture_says_it() {
        let mut plan = Plan::new("t");
        let video = plan.show(media(Kind::Video, Some("YouTube")));
        let step = plan.get(video).unwrap();
        let issue = |level, what: &str| Issue {
            step: video,
            level,
            what: what.to_string(),
        };
        let missing = issue(Level::Problem, "file missing");
        let facts = RowFacts::of(step, Some(&missing), Known::default(), None, false);
        assert_eq!(
            facts.under,
            Under::Wrong(Level::Problem, "file missing".to_string())
        );
        assert_eq!(facts.thumb.state, None);
        let coming = issue(Level::Waiting, "downloading 42%");
        let downloading = Known {
            download: Some(0.42),
            ..Known::default()
        };
        let facts = RowFacts::of(step, Some(&coming), downloading, None, false);
        assert_eq!(
            facts.thumb.state.map(State::words).as_deref(),
            Some("↓ 42%")
        );
        assert_eq!(facts.under, Under::What("YouTube · video".to_string()));
    }

    #[test]
    fn a_result_says_its_size_its_state_and_the_one_warning_that_matters() {
        let lasts = |seconds: u64| Known {
            length: Some(Estimate {
                length: Duration::from_secs(seconds),
                basis: Basis::Known,
            }),
            ..Known::default()
        };
        let row = |resource: &Resource, known: Known, issue: Option<(Level, &str)>| {
            let issue = issue.map(|(level, what)| (level, what.to_string()));
            let facts = ResultFacts::of(resource, known, issue, false);
            let state = facts.thumb.state.map(State::words);
            (facts.thumb.size.clone(), state, facts.under)
        };
        let what = |words: &str| Under::What(words.to_string());

        let video = media(Kind::Video, Some("Himnario"));
        assert_eq!(
            row(&video, lasts(187), None),
            (Some("3:07".into()), None, what("Himnario · video")),
            "a video's length"
        );
        let deck = media(Kind::Presentation, Some("Sermones"));
        let slides = Known {
            pages: Some(12),
            ..Known::default()
        };
        assert_eq!(row(&deck, slides, None).0.as_deref(), Some("12 slides"));
        let pdf = media(Kind::Document, Some("Boletines"));
        let pages = Known {
            pages: Some(8),
            ..Known::default()
        };
        assert_eq!(row(&pdf, pages, None).0.as_deref(), Some("8 pages"));

        let source = lantern_engine::SourceId::new("bible").unwrap();
        let passage = Resource::new(
            lantern_engine::ResourceId::new(&source, "John.3.16-18"),
            "Juan 3:16-18",
            Kind::Text,
            lantern_engine::Location::Inline("Porque de tal manera…".into()),
        )
        .with_subtitle("Reina-Valera 1909")
        .with_meta("verses", "16,17,18");
        assert_eq!(
            row(&passage, Known::default(), None),
            (Some("3 verses".into()), None, what("Reina-Valera 1909")),
            "a passage's verses"
        );

        let screen = Pixels {
            width: 1920,
            height: 1080,
        };
        let small = Known {
            pixels: Some(Pixels {
                width: 800,
                height: 600,
            }),
            screen,
            ..Known::default()
        };
        let picture = media(Kind::Image, Some("Fotos"));
        assert_eq!(
            row(&picture, small, None),
            (
                Some("800×600".into()),
                Some("⚠".into()),
                Under::Soft(screen)
            ),
            "a soft picture warns of it"
        );
        assert_eq!(
            row(&picture, small, Some((Level::Problem, "file missing"))),
            (
                Some("800×600".into()),
                None,
                Under::Wrong(Level::Problem, "file missing".into())
            ),
            "a missing file is the one warning, in red"
        );

        let online = |reach, download| Known {
            reach,
            download,
            ..Known::default()
        };
        let youtube = media(Kind::Video, Some("YouTube"));
        assert_eq!(
            row(
                &youtube,
                online(Reach::Online, Some(0.4)),
                Some((Level::Waiting, "downloading 40%"))
            ),
            (None, Some("↓ 40%".into()), what("YouTube · video")),
            "a download says how far it has come, not that it isn't done"
        );
        assert_eq!(
            row(&youtube, online(Reach::Online, None), None)
                .1
                .as_deref(),
            Some("☁ online")
        );
        assert_eq!(
            row(
                &youtube,
                online(Reach::Online, None),
                Some((Level::Waiting, "not downloaded yet"))
            ),
            (
                None,
                None,
                Under::Wrong(Level::Waiting, "not downloaded yet".into())
            ),
        );
        assert_eq!(
            row(&youtube, online(Reach::Stream, None), None)
                .1
                .as_deref(),
            Some("● live"),
            "a stream needs the internet throughout"
        );

        let shown = ResultFacts::of(&picture, small, None, true);
        assert_eq!(
            (shown.thumb.state.map(State::words), shown.under),
            (Some("● live".into()), Under::Soft(screen)),
            "on stage, it says so, and still warns"
        );
    }

    #[test]
    fn a_row_warns_of_a_soft_picture_unless_something_is_wrong_with_it() {
        let mut plan = Plan::new("t");
        let picture = plan.show(media(Kind::Image, Some("Fixtures")));
        let video = plan.show(media(Kind::Video, Some("Fixtures")));
        let screen = Pixels {
            width: 1920,
            height: 1080,
        };
        let small = Known {
            pixels: Some(Pixels {
                width: 800,
                height: 600,
            }),
            screen,
            ..Known::default()
        };
        let step = plan.get(picture).unwrap();
        let soft = RowFacts::of(step, None, small, None, false);
        assert_eq!(soft.thumb.size.as_deref(), Some("800×600"));
        assert_eq!(soft.thumb.state.map(State::words).as_deref(), Some("⚠"));
        assert_eq!(soft.under, Under::Soft(screen));
        assert_eq!(
            soft_words(screen),
            "Smaller than the screen (1920×1080): it will look soft"
        );
        let lasts = Known {
            length: Some(Estimate {
                length: Duration::from_secs(6),
                basis: Basis::Known,
            }),
            ..small
        };
        let played = RowFacts::of(plan.get(video).unwrap(), None, lasts, None, false);
        assert_eq!(played.thumb.size.as_deref(), Some("0:06"), "its length");
        assert_eq!(played.under, Under::Soft(screen), "a video warns alike");
        let missing = Issue {
            step: picture,
            level: Level::Problem,
            what: "file missing".to_string(),
        };
        let wrong = RowFacts::of(step, Some(&missing), small, None, false);
        assert_eq!(
            wrong.under,
            Under::Wrong(Level::Problem, "file missing".to_string())
        );
        assert_eq!(wrong.thumb.state, None, "the one warning is what's wrong");
    }

    #[test]
    fn progress_says_how_far_in_the_stage_is() {
        use std::num::NonZeroUsize;

        use lantern_engine::{Kind, Location, Playback, ResourceId, SourceId, Timed};

        let resource = |kind| {
            let source = SourceId::new("test").unwrap();
            Resource::new(
                ResourceId::new(&source, "x"),
                "X",
                kind,
                Location::Inline("a".into()),
            )
        };
        let frames = |frame, count| Media::Frames {
            frame,
            count: NonZeroUsize::new(count),
        };
        let (text, done) = progress(&frames(1, 4), &resource(Kind::Text)).unwrap();
        assert_eq!((text.as_str(), done), ("slide 2 of 4", 0.5));
        let (text, _) = progress(&frames(0, 3), &resource(Kind::Document)).unwrap();
        assert_eq!(text, "page 1 of 3");
        assert_eq!(
            progress(&frames(0, 1), &resource(Kind::Text)),
            None,
            "one slide"
        );

        let timed = |duration| {
            Media::Timed(Timed {
                position: Duration::from_secs(82),
                duration,
                playback: Playback::Playing,
            })
        };
        let (text, _) = progress(
            &timed(Some(Duration::from_secs(266))),
            &resource(Kind::Video),
        )
        .unwrap();
        assert_eq!(text, "1:22 / 4:26");
        assert_eq!(
            progress(&timed(None), &resource(Kind::Video)),
            None,
            "length unknown"
        );
    }

    fn head(section: StepId, depth: u8, folded: bool) -> Listed {
        Listed::Head {
            section,
            depth,
            folded,
        }
    }

    #[test]
    fn each_header_comes_before_what_it_holds_and_a_folded_one_alone() {
        use Listed::{Foot, Step as Row};
        let mut plan = Plan::new("Sunday");
        let welcome = plan.push("Welcome", Action::Blank);
        let worship = plan.section("Worship", None);
        let song = plan.push("Song", Action::Blank);
        let hymn = plan.push("Hymn", Action::Blank);
        let word = plan.section("Word", None);
        let reading = plan.push("Reading", Action::Blank);
        let open = |_: StepId| false;
        assert_eq!(
            listed(&plan, &open, &[], &[], None, false),
            [
                Row(welcome),
                head(worship, 0, false),
                Row(song),
                Row(hymn),
                head(word, 0, false),
                Row(reading),
            ]
        );
        // With feet, each section has one under its last row.
        assert_eq!(
            listed(&plan, &open, &[], &[], None, true),
            [
                Row(welcome),
                head(worship, 0, false),
                Row(song),
                Row(hymn),
                Foot(worship),
                head(word, 0, false),
                Row(reading),
                Foot(word),
            ]
        );
        // Folded, its header alone: no foot, and nothing it holds but a
        // step kept in sight.
        let shut = |id: StepId| id == worship;
        assert_eq!(
            listed(&plan, &shut, &[], &[], None, true),
            [
                Row(welcome),
                head(worship, 0, true),
                head(word, 0, false),
                Row(reading),
                Foot(word),
            ]
        );
        assert_eq!(
            listed(&plan, &shut, &[hymn], &[], None, false),
            [
                Row(welcome),
                head(worship, 0, true),
                Row(hymn),
                head(word, 0, false),
                Row(reading),
            ]
        );
    }

    #[test]
    fn an_empty_section_has_its_place_and_one_inside_another_is_a_level_in() {
        use Listed::{Empty, Step as Row};
        let mut plan = Plan::new("Sabbath");
        let notices = plan.section("Notices", None);
        let music = plan.section("Music", None);
        let a = plan.push("A", Action::Blank);
        let hymns = plan.section("Hymns", None);
        let b = plan.push("B", Action::Blank);
        assert!(plan.nest(hymns, true));
        let open = |_: StepId| false;
        assert_eq!(
            listed(&plan, &open, &[], &[], None, false),
            [
                head(notices, 0, false),
                Empty(notices),
                head(music, 0, false),
                Row(a),
                head(hymns, 1, false),
                Row(b),
            ]
        );
        // All it holds carried away, its place shows.
        assert_eq!(
            listed(&plan, &open, &[], &[a, hymns, b], None, false),
            [
                head(notices, 0, false),
                Empty(notices),
                head(music, 0, false),
                Empty(music),
            ]
        );
    }

    #[test]
    fn the_gap_opens_where_what_is_carried_lands_in_or_after_a_section() {
        use Listed::{Empty, Foot, Gap, Step as Row};
        let mut plan = Plan::new("Sunday");
        let welcome = plan.push("Welcome", Action::Blank);
        let notices = plan.section("Notices", None);
        let worship = plan.section("Worship", None);
        let song = plan.push("Song", Action::Blank);
        let word = plan.section("Word", None);
        let reading = plan.push("Reading", Action::Blank);
        let open = |_: StepId| false;
        // Welcome carried. Rest: Notices, Worship, Song, Word, Reading.
        let gap =
            |at: usize, level: u8| listed(&plan, &open, &[], &[welcome], Some((at, level)), true);
        // Under Word's header: first in Word.
        assert_eq!(
            gap(4, 1),
            [
                head(notices, 0, false),
                Empty(notices),
                Foot(notices),
                head(worship, 0, false),
                Row(song),
                Foot(worship),
                head(word, 0, false),
                Gap,
                Row(reading),
                Foot(word),
            ]
        );
        // At Worship's end: in it, before its foot; out of it, after.
        assert_eq!(
            gap(3, 1)[3..7],
            [head(worship, 0, false), Row(song), Gap, Foot(worship)]
        );
        assert_eq!(
            gap(3, 0)[3..7],
            [head(worship, 0, false), Row(song), Foot(worship), Gap]
        );
        // In Notices, which holds nothing, the gap is its place.
        assert_eq!(
            gap(1, 1)[..4],
            [
                head(notices, 0, false),
                Gap,
                Foot(notices),
                head(worship, 0, false)
            ]
        );
        assert_eq!(
            gap(1, 0)[..5],
            [
                head(notices, 0, false),
                Empty(notices),
                Foot(notices),
                Gap,
                head(worship, 0, false)
            ]
        );
    }

    #[test]
    fn the_lit_section_keeps_those_inside_it_and_the_rest_step_back() {
        use crate::track::Light;
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        plan.push("A", Action::Blank);
        let hymns = plan.section("Hymns", None);
        plan.push("B", Action::Blank);
        assert!(plan.nest(hymns, true));
        let sermon = plan.section("Sermon", None);
        plan.push("C", Action::Blank);
        let of = |lit, id| lit_of(&plan, lit, id);
        assert_eq!(of(None, music), Light::Plain);
        assert_eq!(of(Some(music), music), Light::Lit);
        assert_eq!(of(Some(music), hymns), Light::Within);
        assert_eq!(of(Some(music), sermon), Light::Back);
        // Inside it, the one round it steps back.
        assert_eq!(of(Some(hymns), music), Light::Back);
    }

    #[test]
    fn a_sections_line_breaks_at_a_header_inside_it_and_goes_on_after() {
        use Listed::{Foot, Step as Row};
        // Music: A, Hymns [B], C. Hymns ends at B, so C is Music's.
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        let a = plan.push("A", Action::Blank);
        let hymns = plan.section("Hymns", None);
        let b = plan.push("B", Action::Blank);
        let c = plan.push("C", Action::Blank);
        assert!(plan.nest(hymns, true));
        assert!(plan.reach_section(hymns, Some(b)).is_ok());
        let open = |_: StepId| false;
        let rows = listed(&plan, &open, &[], &[], None, false);
        assert_eq!(
            rows,
            [
                head(music, 0, false),
                Row(a),
                head(hymns, 1, false),
                Row(b),
                Row(c)
            ]
        );
        // From each header to its last dot before the next one inside it,
        // and on from the first dot after that one ends.
        assert_eq!(
            stretches(&plan, &rows, &[]),
            [(music, 0, 1), (hymns, 2, 3), (music, 4, 4)]
        );
        // With feet, each runs on into its own, where it is capped.
        let rows = listed(&plan, &open, &[], &[], None, true);
        assert_eq!(rows[4..], [Foot(hymns), Row(c), Foot(music)]);
        assert_eq!(
            stretches(&plan, &rows, &[]),
            [(music, 0, 1), (hymns, 2, 4), (music, 5, 6)]
        );
        // A folded one is a dot on the line round it; one that holds
        // nothing has a line only down to its foot.
        let shut = |id: StepId| id == hymns;
        let rows = listed(&plan, &shut, &[], &[], None, false);
        assert_eq!(stretches(&plan, &rows, &[]), [(music, 0, 3)]);
        let mut empty = Plan::new("Sabbath");
        let notices = empty.section("Notices", None);
        let rows = listed(&empty, &open, &[], &[], None, true);
        assert_eq!(stretches(&empty, &rows, &[]), [(notices, 0, 2)]);
        assert_eq!(
            stretches(&empty, &listed(&empty, &open, &[], &[], None, false), &[]),
            []
        );
    }

    #[test]
    fn a_header_says_its_time_against_its_budget() {
        let at = TimeOfDay::parse("19:15");
        assert_eq!(
            time_words(24 * SECOND, Some(5 * MIN), at, false, false),
            "0:24 of 5:00 · by 19:15"
        );
        assert_eq!(time_words(150 * SECOND, None, None, false, false), "2:30");
        assert_eq!(
            time_words(Duration::ZERO, Some(10 * MIN), None, true, false),
            "empty · 10:00"
        );
        assert_eq!(
            time_words(292 * SECOND, Some(5 * MIN), None, false, true),
            "4:52 of 5:00 · done"
        );
    }

    #[test]
    fn each_span_is_a_lane_further_out_than_what_it_holds() {
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        for label in ["A", "B", "C", "D"] {
            plan.push(label, Action::Blank);
        }
        plan.section("Sermon", None);
        for label in ["E", "F"] {
            plan.push(label, Action::Blank);
        }
        let id = |at: usize| plan.steps()[at].id;
        let again = |plan: &Plan| laid(plan, |flow| flow.repeat != Repeat::Once);
        // Nothing comes again: no lane.
        assert_eq!(again(&plan), vec![]);
        // Music, A, B, C, D, Sermon, E, F. A loop round B and C.
        let (b, c, d, e) = (id(2), id(3), id(4), id(6));
        let inner = plan.loop_over(b, c).unwrap();
        // The section it is in comes again too: one lane further out,
        // from its heading.
        let outer = plan.loop_section(music).unwrap();
        // And a loop across the heading, from D to E, is refused: it
        // would cut the section's.
        assert!(plan.loop_over(d, e).is_err());
        let found = again(&plan);
        let lane = |of: FlowOf| {
            found
                .iter()
                .find(|laid| laid.of == of)
                .map(|laid| laid.lane)
        };
        assert_eq!(lane(FlowOf::Span(inner)), Some(0));
        assert_eq!(lane(FlowOf::Span(outer)), Some(1));
        let section = found
            .iter()
            .find(|laid| laid.of == FlowOf::Span(outer))
            .unwrap();
        assert_eq!(
            (section.first, section.last, section.section),
            (1, 4, Some(music))
        );
        // The plan itself: outermost of all.
        plan.flow.repeat = Repeat::Forever;
        let found = again(&plan);
        assert_eq!(found[0].of, FlowOf::Plan);
        assert_eq!((found[0].first, found[0].last, found[0].lane), (1, 7, 2));
        // Drawn orders are counted apart.
        assert_eq!(laid(&plan, |flow| flow.shuffle), vec![]);
    }
}
