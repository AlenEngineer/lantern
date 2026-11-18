//! The left pane: the plan. Its steps, grouped under section headers that
//! say how each part is keeping to its time, edited in place (see
//! [`crate::plan_edit`]), and behind the Plans button the saved plans.
//!
//! Clicking a step selects it, opens its edit strip and lines it up in the
//! preview; Enter puts it on stage, and Ctrl+Enter adds after it.

use std::time::{Duration, SystemTime};

use iced::widget::{
    Space, button, column, container, mouse_area, row, stack, text, text_input, tooltip,
};
use iced::{Alignment, Element, Fill, Length, Rectangle, mouse};
use lantern_engine::{
    Action, Basis, FadeTime, Flow, FlowOf, Media, Pace, Phase, Plan, Query, Repeat, Resource,
    SectionTiming, Step, StepId, Timing,
};

use crate::dashboard::{NAME_BOX, level_tone, op, unit};
use crate::drag::Half;
use crate::keys::{self, Op};
use crate::length;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL_SMALL, Tone};
use crate::palette::Palette;
use crate::plan_edit::{NewStep, Part, goes_on_when, insertion_mark};
use crate::plans::{self, Entry, Notice, Switch, file_name};
use crate::ready::Issue;
use crate::widgets::{self, Filled, hinted, pill, scrolling, single_line, wide};
use crate::{App, Clicked, FadeEnd, Message, track};

/// The id of the pane's rows, to scroll them.
pub const ROWS: &str = "plan-rows";

/// How far a section's steps sit in from its header: room between the
/// section's own line and its steps' for the line of a loop round some
/// of them.
pub(crate) const INDENT: f32 = 12.0;

/// The field the plan's target time is typed in.
pub(crate) const TARGET_BOX: &str = "plan-target";

/// The rail beside the steps: how wide its column is, its dots, its
/// line, and how far down a row its dot sits, level with the step's name.
const RAIL: f32 = 20.0;
const RAIL_DOT: f32 = 10.0;
const RAIL_LINE: f32 = 2.0;
const RAIL_TOP: f32 = 15.0;
/// How far the ring round the dot of the step that is up stands off it.
const RAIL_RING: f32 = 4.0;

/// About how tall a step's row is, and each line under it, and the gap a
/// drag opens: for what is cut to a height on its way open or closed. A
/// little over is right; under would cut a row that should show whole.
const ROW_TALL: f32 = 44.0;
const PART_TALL: f32 = 22.0;
const GAP_TALL: f32 = 48.0;
/// How tall the place for one carried step is, how many of them are
/// drawn at most, and how tall the place for a carried section is.
const PLACE_TALL: f32 = 36.0;
const PLACES: usize = 6;
const BLOCK_TALL: f32 = 60.0;

/// The plan's own row: a right click on it opens the ring of how the
/// whole plan moves, where the plan can change (`can`).
fn plans_own<'a>(row: Element<'a, Message>, can: bool) -> Element<'a, Message> {
    if !can {
        return row;
    }
    mouse_area(row)
        .on_right_press(Message::Rail(crate::rail::Act::Ring(
            crate::rail::RingOn::All(None),
        )))
        .into()
}

/// What the place of a file carried in from the library goes by, among the
/// places of steps: no step of a plan has it.
fn held_place(n: usize) -> StepId {
    serde_json::from_value(serde_json::Value::from(u64::MAX - n as u64))
        .expect("a step id is a number")
}

/// The sections that are brackets in `plan` (as `bracketed` says), each
/// a lane further out than those it holds.
fn lay_brackets(plan: &Plan, bracketed: &dyn Fn(&Plan, usize) -> bool) -> Vec<Braced> {
    let mut braced: Vec<Braced> = (0..plan.len())
        .filter(|at| bracketed(plan, *at))
        .map(|at| Braced {
            at,
            end: plan.group_end(at),
            lane: 0,
        })
        .collect();
    for at in (0..braced.len()).rev() {
        let (outer, inner) = braced.split_at_mut(at + 1);
        let this = &mut outer[at];
        this.lane = inner
            .iter()
            .filter(|held| held.end <= this.end)
            .map(|held| held.lane + 1)
            .max()
            .unwrap_or(0);
    }
    braced
}

/// A section that is a bracket beside its steps: where its heading is,
/// where what it holds ends, and how far out its bracket is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Braced {
    at: usize,
    end: usize,
    lane: u8,
}

/// The brackets beside the steps: which sections they are, how many
/// lanes of them, and whether one needs room right of its brace.
#[derive(Debug, Clone, Copy)]
struct Side<'a> {
    braced: &'a [Braced],
    lanes: u8,
    wide: bool,
    /// How much wider the innermost brackets' room is while a picked
    /// section's card is in it.
    more: f32,
}

/// How tall the foot of a bracket is, to be pulled.
const FOOT: f32 = 16.0;

/// How wide the card of a picked section is: a little more than the
/// room its name had.
const CARD: f32 = track::LABEL + 30.0;

/// How wide the steps' room is kept, for their dots, names and times,
/// before a section that holds only sections gives up the room for its
/// name.
const NAMES_ROOM: f32 = 150.0;

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
#[derive(Debug, Clone, Copy)]
pub(crate) struct Lanes<'a> {
    curl: f32,
    drawn: f32,
    loops: &'a [Laid],
    orders: &'a [Laid],
    /// How many lanes each side has room for.
    loop_lanes: u8,
    order_lanes: u8,
}

/// `row` with the room there is between two rows of the plan under it.
fn apart<'a>(row: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(row)
        .padding(iced::Padding::default().bottom(2.0))
        .into()
}

/// Where a step is in the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Done,
    Current,
    Next,
    Later,
}

impl App {
    pub(crate) fn plan_pane(&self, timing: &Timing) -> Element<'_, Message> {
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
        // While planning, the rail's three marks, named once at the foot.
        if self.mode == crate::mode::Mode::Plan
            && matches!(
                self.plans.mode,
                plans::Mode::Steps | plans::Mode::Renaming(_)
            )
        {
            let quiet = p.on_surface_variant;
            let named = |mark: Element<'static, Message>, name: &'static str| {
                row![mark, text(name).size(META).color(quiet)]
                    .spacing(6)
                    .align_y(Alignment::Center)
            };
            pane = pane.push(
                container(
                    row![
                        named(
                            look::icon(Icon::Joined, 13.0, p.primary).into(),
                            "by itself"
                        ),
                        named(look::icon(Icon::Pause, 13.0, quiet).into(), "waits"),
                        named(look::ring(0.4, 13.0, p.tertiary).into(), "downloading"),
                    ]
                    .spacing(16),
                )
                .padding([0, 8]),
            );
        }
        pane = pane.push(container(status).padding([0, 8]));
        let pane = pane.padding([16, 12]).height(Fill);
        iced::widget::stack![pane, self.refusal_over()]
            .width(self.plan_width())
            .height(Fill)
            .into()
    }

    /// The Plans button, the plan's name (click to rename), and where the
    /// service is: "3 of 7".
    pub(crate) fn plan_header(&self, plan: &Plan) -> Element<'_, Message> {
        let p = self.palette;
        // The list of plans is the pane's home: its header is its own, not
        // the open plan's name and count over a list of other plans.
        if matches!(self.plans.mode, plans::Mode::Browsing(_)) {
            return container(text("Plans").size(HEADING).font(BOLD).color(p.primary))
                .padding([4, 8])
                .height(PILL_SMALL)
                .align_y(Alignment::Center)
                .into();
        }
        // The way back to the list.
        let plans_button = tooltip(
            button(text("‹").size(HEADING).color(p.on_surface_variant))
                .padding([2, 10])
                .on_press(op(Op::Open))
                .style(look::row(p, Tone::Plain)),
            container(text(keys::with_key("All plans", Op::Open)).size(META))
                .padding([4, 8])
                .style(look::tip(p)),
            tooltip::Position::Bottom,
        )
        .delay(look::TIP_DELAY);
        let name: Element<'_, Message> = match &self.plans.mode {
            plans::Mode::Renaming(draft) => text_input("Plan name", draft)
                .id(NAME_BOX)
                .on_input(Message::PlanName)
                .on_submit(Message::SubmitPlanName)
                .size(BODY)
                .padding([6, 10])
                .style(look::input(p))
                .into(),
            // A long name is cut at its edge, not drawn over the count.
            // It opens the plan's own settings, and is lit while they
            // are open; in a run it is only a name.
            _ => {
                let open = self.plan_panel;
                let name = container(
                    single_line(plan.title.clone())
                        .size(HEADING)
                        .font(BOLD)
                        .color(p.primary),
                )
                .clip(true);
                let running = self.mode == crate::mode::Mode::Run;
                let mut inside = row![name].spacing(8).align_y(Alignment::Center);
                if !running {
                    inside = inside.push(look::icon(Icon::Tune, 14.0, p.primary));
                }
                tooltip(
                    button(inside)
                        .padding([4, 10])
                        .on_press_maybe((!running).then_some(Message::PlanSettings))
                        .style(look::pill(
                            p,
                            if open { Tone::Selected } else { Tone::Plain },
                        )),
                    container(text("This plan: name, time, media, look, copy, delete").size(META))
                        .padding([4, 8])
                        .style(look::tip(p)),
                    tooltip::Position::Bottom,
                )
                .delay(look::TIP_DELAY)
                .into()
            }
        };
        // The name has the header's room. Where the service is ("3 of 7")
        // shows only once it's running; how many steps there are is plain
        // from the list.
        let items: Vec<&Step> = plan.steps().iter().filter(|s| !s.is_section()).collect();
        let at = self
            .engine
            .0
            .current_step()
            .and_then(|current| items.iter().position(|s| s.id == current.id));
        // In a run the plan can't be left or managed: the way back to the
        // list and the plan's menu come back when the service ends.
        let running = self.mode == crate::mode::Mode::Run;
        let mut header = match running {
            true => row![name],
            false => row![plans_button, name],
        };
        if let Some(at) = at {
            header = header.push(
                text(format!("{} of {}", at + 1, items.len()))
                    .size(META)
                    .color(p.on_surface_variant),
            );
        }
        if self.plans.is_dirty(plan) {
            header = header.push(hinted(
                p,
                pill(p, "Save", Tone::Selected, PILL_SMALL, Some(op(Op::Save))),
                keys::with_key("Save the plan", Op::Save),
                tooltip::Position::Bottom,
            ));
        } else if let Some(shown) = self.motion.tick() {
            // Saved: Save is a tick for a moment, fading.
            header = header.push(
                container(look::icon(
                    Icon::Check,
                    14.0,
                    iced::Color {
                        a: shown,
                        ..p.primary
                    },
                ))
                .padding([3, 10]),
            );
        }
        header.spacing(6).align_y(Alignment::Center).into()
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
        // A file carried in from the library opens the same gap.
        let held = self.carried.as_ref().filter(|held| held.moving);
        let landing = moving
            .and_then(|drag| drag.landing)
            .or_else(|| self.carry_landing(plan));
        // It opens as it comes, so the rows under it are moved aside and
        // not thrown there.
        let opening = self.motion.gap();
        // The gap it just left closes at the same pace, so the rows in
        // between slide over instead of jumping.
        let closing = self.motion.closing_gap();
        // The plan without what is carried: where it lands is counted
        // in this.
        let rest_steps: Vec<&Step> = plan
            .steps()
            .iter()
            .filter(|step| !carried.contains(&step.id))
            .collect();
        // How far in it would land: a section where the pointer has taken
        // it, a step under whatever it lands in.
        let gap_level = match (moving, landing) {
            (Some(drag), _) => drag.level,
            (None, Some(at)) => crate::drag::level_at(&rest_steps, at),
            (None, None) => 0,
        };
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
        // place on the line and room for the brackets, so the section
        // they would be in takes them in and a loop or a drawn order
        // they bring is drawn beside them. For a section, one place with
        // a bracket of its own. For a file from the library, as before.
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
            (0, Some(_)) => BLOCK_TALL + 4.0 + ruler.map_or(0.0, |_| RULER_TALL),
            (places, _) => PLACE_TALL * places as f32,
        };
        let closing_gap = move |left: f32| Space::new().height(gap_tall * left);
        let gap = |side_wide: f32, lanes: Lanes<'_>| -> Element<'_, Message> {
            let dashes = |tall: f32| {
                crate::drawn::dashed(
                    container(Space::new().width(Fill).height(tall)).style(move |_| {
                        container::Style {
                            background: Some(Palette::tint(p.primary, 0.08).into()),
                            border: iced::Border {
                                radius: 9.0.into(),
                                ..iced::Border::default()
                            },
                            ..container::Style::default()
                        }
                    }),
                    Palette::tint(p.primary, 0.7),
                    9.0,
                )
            };
            let beside = |id: StepId, dot: bool| -> Element<'_, Message> {
                // The same room a step's rail takes, with where its dot
                // would be.
                let mark: Element<'_, Message> = match dot {
                    true => column![
                        Space::new().height(RAIL_TOP - 2.0),
                        track::seen(
                            track::Key::Dot(id),
                            &self.places,
                            Space::new().width(RAIL_DOT).height(RAIL_DOT)
                        ),
                    ]
                    .width(RAIL)
                    .align_x(Alignment::Center)
                    .into(),
                    false => Space::new().width(RAIL).into(),
                };
                track::seen(
                    track::Key::Row(id),
                    &self.places,
                    row![
                        Space::new().width(lanes.curl),
                        mark,
                        Space::new().width(lanes.drawn)
                    ]
                    .height(Fill),
                )
            };
            let room = |id: StepId| {
                track::seen(
                    track::Key::Side(id),
                    &self.places,
                    Space::new().width(side_wide).height(Fill),
                )
            };
            let slot: Element<'_, Message> = match (stepwise.is_empty(), blockwise) {
                (false, _) => column(stepwise.iter().map(|id| {
                    row![
                        beside(*id, true),
                        container(dashes(PLACE_TALL - 8.0)).padding([4, 0]),
                        room(*id)
                    ]
                    .height(PLACE_TALL)
                    .into()
                }))
                .into(),
                (true, Some(section)) => {
                    let mut block = column![
                        row![
                            beside(section, false),
                            container(dashes(BLOCK_TALL - 4.0)).padding([2, 0]),
                            room(section)
                        ]
                        .height(BLOCK_TALL)
                    ]
                    .spacing(4);
                    if let Some((at, (least, most))) = ruler {
                        block = block.push(
                            container(level_ruler(p, at, least, most))
                                .padding(iced::Padding::default().left(gap_in)),
                        );
                    }
                    block.into()
                }
                (true, None) => container(drop_gap(
                    p,
                    held.map(|held| held.file.title.clone()).unwrap_or_default(),
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
        // The section it would be a part of: lit, with a line down the
        // side of all it holds, the gap too.
        let host = landing.and_then(|at| crate::drag::host(&rest_steps, at, gap_level));
        let host_id = host.map(|(start, _)| rest_steps[start].id);
        let host_lit = self.motion.host_lit();
        let host_in = host.map_or(0.0, |(start, _)| {
            INDENT * f32::from(rest_steps[start].depth().unwrap_or(0))
        });
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
        // Every section is a bracket beside what it holds, unless one
        // folded round it hides it. Each bracket is a lane further out
        // than those it holds.
        // Folded by hand, or over in a service: its steps are gathered
        // into one row beside its bracket.
        let shut = |id: StepId| self.shut(timing, id);
        let bracketed = |plan: &Plan, at: usize| {
            let step = &plan.steps()[at];
            step.is_section() && !plan.within(at).iter().any(|id| *id != step.id && shut(*id))
        };
        let mut braced = lay_brackets(plan, &bracketed);
        // A bracket's foot being pulled: the brackets as letting go would
        // have them (no row moves, so they line up with the rows as they
        // are); one that can't go there says so in the colour of what is
        // wrong.
        let foot_refused = match self.foot {
            Some((section, Some(last))) => {
                let mut after = plan.clone();
                match after.reach_section(section, Some(last)) {
                    Ok(()) => {
                        braced = lay_brackets(&after, &bracketed);
                        None
                    }
                    Err(_) => Some(section),
                }
            }
            _ => None,
        };
        let footed = self.foot.map(|(section, _)| section);
        // A section that is carried is laid out as letting go would have
        // it: a dry run of the drop says which sections it would be in,
        // which it would take into itself, and how far out each bracket
        // is then.
        let dropped = moving
            .filter(|drag| drag.is_section())
            .and_then(|drag| Some((drag, drag.landing?)))
            .and_then(|(drag, at)| {
                let mut after = plan.clone();
                let among = drag.among(plan);
                crate::drag::place_section(&mut after, &drag.carried, at, drag.level, among)
                    .then_some((drag, after))
            });
        let mut carried_lane = 0;
        let mut taken: Vec<StepId> = Vec::new();
        let mut holders: Vec<StepId> = Vec::new();
        if let Some((drag, after)) = &dropped {
            let lanes: std::collections::HashMap<StepId, u8> = lay_brackets(after, &bracketed)
                .into_iter()
                .map(|laid| (after.steps()[laid.at].id, laid.lane))
                .collect();
            braced.retain(|laid| !drag.carried.contains(&plan.steps()[laid.at].id));
            for laid in &mut braced {
                if let Some(lane) = lanes.get(&plan.steps()[laid.at].id) {
                    laid.lane = *lane;
                }
            }
            carried_lane = lanes.get(&drag.step).copied().unwrap_or(0);
            if let Some(at) = after.position(drag.step) {
                holders = after.within(at).into_iter().rev().collect();
                taken = after.steps()[at + 1..after.group_end(at)]
                    .iter()
                    .map(|step| step.id)
                    .filter(|id| !drag.carried.contains(id))
                    .collect();
            }
        }
        let side_lanes = braced
            .iter()
            .map(|braced| braced.lane + 1)
            .chain(dropped.as_ref().map(|_| carried_lane + 1))
            .max()
            .unwrap_or(0);
        // The steps that are a section's own: in it and in no section
        // inside it. A section that holds others writes its name beside
        // the longest run of those; one with none needs room right of
        // its brace.
        let own_of = |outer: &Braced| {
            // What is carried is not where it was.
            let own = |at: &usize| {
                !plan.steps()[*at].is_section()
                    && !carried.contains(&plan.steps()[*at].id)
                    && !braced.iter().any(|inner| {
                        inner.at > outer.at
                            && inner.end <= outer.end
                            && (inner.at..inner.end).contains(at)
                    })
            };
            // The longest run of them, the first of the longest.
            let mut best: Option<(usize, usize)> = None;
            let mut at = outer.at + 1;
            while at < outer.end {
                match own(&at) {
                    true => {
                        let last = (at..outer.end).take_while(own).last().unwrap_or(at);
                        if best.is_none_or(|(from, to)| last - at > to - from) {
                            best = Some((at, last));
                        }
                        at = last + 1;
                    }
                    false => at += 1,
                }
            }
            best
        };
        // That room is given only while the steps keep enough of theirs
        // for their names: without it, such a section is its brace alone,
        // its name in its card.
        let roomy = self
            .places
            .borrow()
            .get(&track::Key::Canvas)
            .is_none_or(|canvas| canvas.width - track::side_room(side_lanes, true) >= NAMES_ROOM);
        let side_wide = roomy
            && braced
                .iter()
                .any(|braced| braced.lane > 0 && own_of(braced).is_none());
        // A picked section's card takes the room its name had, and more:
        // the steps give way to it, so it covers none of them.
        let carded = braced
            .iter()
            .any(|braced| self.selected == Some(plan.steps()[braced.at].id));
        let side = Side {
            braced: &braced,
            lanes: side_lanes,
            wide: side_wide,
            more: match carded {
                true => CARD - track::LABEL,
                false => 0.0,
            },
        };
        let mut stops: Vec<track::Stop> = Vec::new();
        // The row the place for what is carried was put before, where
        // there is one.
        let mut gap_at: Option<usize> = None;
        // Every row in sight: a press on one of them is not on nothing.
        let mut whole: Vec<StepId> = Vec::new();
        // The sections with nothing in it that have a place in sight:
        // a bracket round them runs beside those too.
        let mut placed: Vec<usize> = Vec::new();
        // The folded sections in sight: each one row for all it holds.
        let mut gathered: Vec<usize> = Vec::new();
        let mut shown: Vec<usize> = Vec::new();
        // While planning, the pane is a tree: the open plan, its sections
        // and their steps, and under them the other plans.
        let tree = self.mode == crate::mode::Mode::Plan;
        let running = self.mode == crate::mode::Mode::Run;
        // A right click on the plan's own row opens the ring of how the
        // whole plan moves, where the plan can change.
        let can = self.can_edit();
        let loops = laid(plan, |flow| flow.repeat != Repeat::Once);
        let orders = laid(plan, |flow| flow.shuffle);
        // While planning one of each is always there: a pull can draw
        // either line, and the rows must not shift as it begins.
        let least = u8::from(tree && can);
        let deepest = |of: &[Laid]| of.iter().map(|laid| laid.lane + 1).max().unwrap_or(0);
        let (loop_lanes, order_lanes) = (deepest(&loops).max(least), deepest(&orders).max(least));
        let lanes = Lanes {
            curl: track::loop_room(loop_lanes) - RAIL / 2.0,
            drawn: track::order_room(order_lanes) - RAIL / 2.0,
            loops: &loops,
            orders: &orders,
            loop_lanes,
            order_lanes,
        };
        if tree {
            rows = rows.push(plans_own(self.time_row(plan, timing), can));
        } else if running {
            rows = rows.push(plans_own(self.run_time_row(plan, timing), can));
        }
        // In a service with a start time, a section still to come says
        // when it is due.
        let due = plan.starts.filter(|_| running).map(|starts| {
            let schedule = lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step));
            move |id| schedule.start_of(id).map(|after| starts.plus(after))
        });
        // Where a fill's proposed rows go: the end of its section, as far
        // in as that section's steps.
        let fill_of = self
            .filling
            .as_ref()
            .and_then(|fill| plan.position(fill.section));
        let fill_at = fill_of.map(|at| plan.group_end(at));
        let fill_indent = fill_of.map_or(0.0, |at| {
            INDENT * f32::from(plan.level(at) + 1) + lanes.curl
        });
        // The rows of the section something would land in are kept apart
        // while they are gathered, to have one line down their side.
        let mut hosting: Option<iced::widget::Column<'_, Message>> = None;
        // The steps and what sits among them, with no room between two:
        // each brings its own under it, but for a step's rail, which runs
        // the whole of its row so the line down the dots is unbroken.
        let mut list = column![];
        macro_rules! put_whole {
            ($row:expr $(,)?) => {
                match hosting.take() {
                    Some(held) => hosting = Some(held.push($row)),
                    None => list = list.push($row),
                }
            };
        }
        macro_rules! put {
            ($row:expr $(,)?) => {
                put_whole!(apart($row))
            };
        }
        // Up to where rows are folded away with the section that holds
        // them.
        let mut hidden_to = 0;
        let mut rest = 0;
        for (index, step) in plan.steps().iter().enumerate() {
            if carried.contains(&step.id) {
                continue;
            }
            if landing == Some(rest) {
                put!(gap(
                    track::side_room(side.lanes, side.wide) + side.more,
                    lanes
                ));
                gap_at = Some(index);
            }
            if let Some((_, left)) = closing.filter(|(at, _)| *at == rest) {
                put!(closing_gap(left));
            }
            // What the section it would be a part of holds ends here.
            if host.is_some_and(|(_, end)| end == rest)
                && let Some(held) = hosting.take()
            {
                list = list.push(hosted(p, held, host_in, host_lit));
            }
            if host.is_some_and(|(start, _)| start == rest) {
                hosting = Some(column![]);
            }
            rest += 1;
            let selected = self.selected == Some(step.id) && moving.is_none();
            // What a fill proposes sits at the end of its section,
            // before whatever comes after it.
            if fill_at == Some(index)
                && let Some(proposed) = self.fill_rows(fill_indent)
            {
                put!(track::seen(track::Key::Fill, &places, proposed));
            }
            let hidden = index < hidden_to;
            if step.is_section() {
                // Folded away with the section it is in.
                if hidden {
                    continue;
                }
                // In a service, a section that is over folds away to its
                // one row: what it was, and how long it took.
                let over = self.over(timing, step.id);
                let folded = self.shut(timing, step.id);
                if folded {
                    hidden_to = plan.group_end(index);
                }
                // A bracket beside its steps: no row. One folded has its
                // steps gathered into one row; one with nothing in it has
                // a place where its steps will be.
                let wide = track::side_room(side.lanes, side.wide) + side.more;
                if folded {
                    whole.push(step.id);
                    placed.push(index);
                    gathered.push(index);
                    put!(self.folded_place(plan, step, over, lanes, wide, &places));
                    stops.push(track::Stop {
                        id: step.id,
                        dot: track::Dot::Stack {
                            color: match over {
                                true => p.outline,
                                false => p.primary,
                            },
                            solid: over,
                        },
                        line: None,
                        next: None,
                        back: None,
                    });
                } else if plan.group_end(index) == index + 1 {
                    whole.push(step.id);
                    placed.push(index);
                    put!(self.empty_place(step.id, lanes, wide, &places));
                }
                if selected && adds {
                    put!(insertion_mark(p, INDENT));
                }
                continue;
            }
            // Folded away with its section, unless it is the step the
            // service is at or the one being worked on.
            if hidden && Some(step.id) != current && !selected {
                continue;
            }
            // Rows just taken away from before this one close up.
            if let Some((rows, left)) = self.motion.gone_before(Some(step.id)) {
                put!(Space::new().height(ROW_TALL * rows as f32 * left));
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
            let mut row = column![self.draggable(
                step.id,
                self.step_row(step, place, issue, &noted, picked_end == Some(step.id))
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
            let parts = self.step_parts(step, place == Place::Current);
            let tall = ROW_TALL + parts.len() as f32 * PART_TALL;
            for part in parts {
                row = row.push(part);
            }
            // The rail runs down beside the step and what is inside it.
            // The rail is as tall as the row and the room under it, so
            // its line meets the next row's.
            let (rail, mut stop) = self.rail(plan, index, place, lanes, &places);
            // A step that was not let go where it was carried shakes.
            let aside = alarm
                .filter(|_| self.alarm.0.contains(&step.id))
                .map_or(0.0, f32::abs);
            if aside > 0.0 {
                stop.dot = track::Dot::Would(alarm_ink);
            }
            let row = row![
                Space::new().width(aside),
                rail,
                apart(row),
                self.side(plan, index, side, &places)
            ]
            .height(Length::Shrink);
            whole.push(step.id);
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
            put_whole!(row);
            stops.push(stop);
            shown.push(index);
        }
        if let Some((rows, left)) = self.motion.gone_before(None) {
            put!(Space::new().height(ROW_TALL * rows as f32 * left));
        }
        if fill_at == Some(plan.len())
            && let Some(proposed) = self.fill_rows(fill_indent)
        {
            put!(track::seen(track::Key::Fill, &places, proposed));
        }
        if landing == Some(rest) {
            put!(gap(
                track::side_room(side.lanes, side.wide) + side.more,
                lanes
            ));
            gap_at = Some(plan.len());
        }
        if let Some((_, left)) = closing.filter(|(at, _)| *at == rest) {
            put!(closing_gap(left));
        }
        if let Some(held) = hosting.take() {
            list = list.push(hosted(p, held, host_in, host_lit));
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
        // The last row adds a step that isn't a search result; it opens
        // on the kinds there are.
        if moving.is_none() && self.can_edit() {
            // An empty plan's + breathes a few times: it is where to begin.
            let breath = match plan.is_empty() && self.adding.is_none() {
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
                if self.adding.is_some() { "−" } else { "+" },
                String::new(),
                Message::ToggleAdd,
                breath,
            ));
            if let Some(on) = self.adding {
                // The arrows move among the kinds; Enter adds the lit one.
                let kinds = NewStep::ALL.iter().enumerate().map(|(n, (kind, label))| {
                    let tone = if n == on { Tone::Selected } else { Tone::Plain };
                    pill(p, label, tone, PILL_SMALL, Some(Message::AddStep(*kind))).into()
                });
                rows = rows.push(
                    container(row(kinds).spacing(4)).padding(iced::Padding::default().left(34.0)),
                );
            }
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
        if gap_at.is_some() && !stepwise.is_empty() {
            let ink = Palette::tint(p.primary, 0.7);
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
        // The sections' brackets, each beside the first to the last of
        // its steps in sight, with its name and its times.
        // The sections what is carried would be in, innermost first: for
        // steps, where they would land and all that is around it; for a
        // section, the one it would be a part of.
        let into: Vec<StepId> = match moving {
            Some(drag) if !drag.is_section() => drag
                .landing
                .and_then(|at| self.carry_place(plan, drag, at).section)
                .map(|section| {
                    let around = plan.position(section).map(|at| plan.within(at));
                    std::iter::once(section)
                        .chain(around.unwrap_or_default().into_iter().rev())
                        .collect()
                })
                .unwrap_or_default(),
            // The sections the dry run of the drop has it in.
            Some(_) => holders.clone(),
            // A file carried in: the section it would be a part of, and
            // those round that.
            None => host_id
                .filter(|_| held.is_some())
                .map(|host| {
                    let around = plan.position(host).map(|at| plan.within(at));
                    std::iter::once(host)
                        .chain(around.unwrap_or_default().into_iter().rev())
                        .collect()
                })
                .unwrap_or_default(),
        };
        // The first and last of the places that were opened.
        let place_keys = match (stepwise.first(), stepwise.last(), blockwise) {
            (Some(first), Some(last), _) => Some((*first, *last)),
            (_, _, Some(section)) => Some((section, section)),
            _ => None,
        };
        // Every row a bracket can run beside: the steps, and the places
        // of sections with nothing in them.
        let mut beside: Vec<usize> = shown.iter().chain(&placed).copied().collect();
        beside.sort_unstable();
        let mut brackets: Vec<track::Bracket> = braced
            .iter()
            .filter_map(|braced| {
                let step = &plan.steps()[braced.at];
                let mut inside = beside
                    .iter()
                    .copied()
                    .filter(|at| (braced.at + 1..braced.end).contains(at));
                let picked = self.selected == Some(step.id);
                // Folded: beside its one row, with its name, and how long
                // it took against its time when it is over, else its time
                // and what it would take.
                if shut(step.id) {
                    let goal = match step.action {
                        Action::Section { budget, .. } => budget.filter(|budget| !budget.is_zero()),
                        _ => None,
                    };
                    let section = timing.sections.iter().find(|s| s.id == step.id);
                    let mut lines = Vec::new();
                    match section.map(|section| section.phase) {
                        Some(Phase::Done { took }) => {
                            let took = took.unwrap_or_default();
                            let late = goal.is_some_and(|goal| took > goal);
                            lines.push((
                                track::Sign::Sum,
                                match goal {
                                    Some(goal) => {
                                        format!(
                                            "{} of {}",
                                            length::clock(took),
                                            length::clock(goal)
                                        )
                                    }
                                    None => length::clock(took),
                                },
                                if late { p.error } else { p.outline },
                            ));
                        }
                        _ => {
                            if let Some(goal) = goal {
                                lines.push((
                                    track::Sign::Goal,
                                    length::clock(goal),
                                    p.on_surface_variant,
                                ));
                            }
                            lines.push((
                                track::Sign::Sum,
                                length::clock(self.planned(plan, step.id)),
                                p.on_surface_variant,
                            ));
                        }
                    }
                    return Some(track::Bracket {
                        id: step.id,
                        first: step.id,
                        last: step.id,
                        offset: track::brace_at(0),
                        beside: None,
                        color: match self.over(timing, step.id) {
                            true => p.outline,
                            false => p.primary,
                        },
                        name: step.label.clone(),
                        lines,
                        strong: picked,
                        bare: picked,
                        place: false,
                        marks: crate::notes::numbers_on(
                            plan,
                            lantern_engine::NoteOn::Step(step.id),
                        )
                        .into_iter()
                        .map(|(_, n)| n)
                        .collect(),
                        bulge: 0.0,
                    });
                }
                // With nothing in it: beside its place, with its name and
                // the time it is given.
                if braced.end == braced.at + 1 {
                    let goal = match step.action {
                        Action::Section { budget, .. } => budget.filter(|budget| !budget.is_zero()),
                        _ => None,
                    };
                    // What is carried would go into it: its bracket takes in
                    // the place opened for it, under its own.
                    let last = match (gap_at, place_keys) {
                        (Some(_), Some((_, to))) if into.first() == Some(&step.id) => to,
                        _ => step.id,
                    };
                    return Some(track::Bracket {
                        id: step.id,
                        first: step.id,
                        last,
                        offset: track::brace_at(0),
                        beside: None,
                        color: p.primary,
                        name: step.label.clone(),
                        lines: goal
                            .map(|goal| {
                                (track::Sign::Goal, length::clock(goal), p.on_surface_variant)
                            })
                            .into_iter()
                            .collect(),
                        strong: picked
                            || host_id == Some(step.id)
                            || into.first() == Some(&step.id),
                        bare: picked,
                        place: false,
                        marks: crate::notes::numbers_on(
                            plan,
                            lantern_engine::NoteOn::Step(step.id),
                        )
                        .into_iter()
                        .map(|(_, n)| n)
                        .collect(),
                        bulge: 0.0,
                    });
                }
                let first = inside.next()?;
                let last = inside.next_back().unwrap_or(first);
                // The section what is carried would be in takes its
                // place in: its bracket runs from there, or to there,
                // where the place is before its first step or after its
                // last.
                let takes = into.contains(&step.id);
                let (first_key, last_key) = match (gap_at.filter(|_| takes), place_keys) {
                    (Some(gap), Some((from, _))) if gap <= first => (from, id_at(last)),
                    (Some(gap), Some((_, to))) if gap > last => (id_at(first), to),
                    _ => (id_at(first), id_at(last)),
                };
                let section = timing.sections.iter().find(|s| s.id == step.id);
                let budget = match step.action {
                    Action::Section { budget, .. } => budget.filter(|budget| !budget.is_zero()),
                    _ => None,
                };
                let phase = section.map_or(
                    Phase::Coming {
                        planned: self.planned(plan, step.id),
                    },
                    |section| section.phase,
                );
                // The time it takes, against the time it is given.
                let (sum, late, on) = match phase {
                    Phase::Coming { planned } => (
                        planned,
                        budget.is_some_and(|budget| planned > budget),
                        false,
                    ),
                    Phase::Running { elapsed, expected } => (
                        elapsed,
                        budget.is_some_and(|budget| expected > budget),
                        true,
                    ),
                    Phase::Done { took } => (took.unwrap_or_default(), false, false),
                };
                // Over in a service and opened again: grey, as folded.
                let past = self.over(timing, step.id);
                let mut lines = Vec::new();
                if let Some(budget) = budget {
                    lines.push((
                        track::Sign::Goal,
                        length::clock(budget),
                        p.on_surface_variant,
                    ));
                }
                // Just tipped over its time: the sum turns red as it goes.
                let tipped = self.motion.tipped(step.id);
                lines.push((
                    track::Sign::Sum,
                    length::clock(sum),
                    match (late, on, tipped) {
                        (true, _, Some(t)) => crate::motion::blend(p.on_surface, p.error, t),
                        (true, _, None) => p.error,
                        (false, true, _) => p.tertiary,
                        (false, false, _) if past => p.outline,
                        (false, false, _) => p.on_surface,
                    },
                ));
                // The time of day it should be over by, in the colour of
                // what is wrong where it is expected to end after; else, in
                // a service, when it is due.
                match step.done_by() {
                    Some(by) => lines.push((
                        track::Sign::Clock,
                        by.to_string(),
                        match section.and_then(SectionTiming::late) {
                            Some(_) => p.error,
                            None => p.on_surface_variant,
                        },
                    )),
                    None => {
                        if let Some(due) = due.as_ref().and_then(|due| due(step.id)).filter(|_| !on)
                        {
                            lines.push((track::Sign::Clock, due.to_string(), p.on_surface_variant));
                        }
                    }
                }
                let picked = self.selected == Some(step.id);
                let beside = Some(braced)
                    .filter(|braced| braced.lane > 0)
                    .and_then(own_of)
                    .map(|(first, last)| (id_at(first), id_at(last)));
                Some(track::Bracket {
                    id: step.id,
                    first: first_key,
                    last: last_key,
                    offset: track::brace_at(braced.lane)
                        + match braced.lane {
                            0 => 0.0,
                            _ => side.more,
                        },
                    // One that holds others writes beside its own steps.
                    beside,
                    color: match on {
                        _ if foot_refused == Some(step.id) => p.error,
                        _ if past => p.outline,
                        true => p.tertiary,
                        false => p.primary,
                    },
                    name: step.label.clone(),
                    lines,
                    strong: picked
                        || self.picked.contains(&step.id)
                        || host_id == Some(step.id)
                        || into.first() == Some(&step.id)
                        || footed == Some(step.id),
                    bare: picked || (braced.lane > 0 && beside.is_none() && !side.wide),
                    place: false,
                    marks: crate::notes::numbers_on(plan, lantern_engine::NoteOn::Step(step.id))
                        .into_iter()
                        .map(|(_, n)| n)
                        .collect(),
                    bulge: self
                        .motion
                        .tipped(step.id)
                        .map_or(0.0, |t| (t * std::f32::consts::PI).sin()),
                })
            })
            .collect();
        // A section that is carried has a bracket of its own at its
        // place, dashed, with its name faint beside it.
        if let Some(section) = blockwise.filter(|_| gap_at.is_some())
            && let Some(step) = plan.get(section)
        {
            let budget = match step.action {
                Action::Section { budget, .. } => budget.filter(|budget| !budget.is_zero()),
                _ => None,
            };
            // Down past what it would take into itself.
            let last = taken
                .iter()
                .rev()
                .find(|id| plan.position(**id).is_some_and(|at| beside.contains(&at)))
                .copied()
                .unwrap_or(section);
            brackets.push(track::Bracket {
                id: section,
                first: section,
                last,
                offset: track::brace_at(carried_lane),
                beside: None,
                color: p.primary,
                name: step.label.clone(),
                lines: budget
                    .map(|budget| {
                        (
                            track::Sign::Goal,
                            length::clock(budget),
                            p.on_surface_variant,
                        )
                    })
                    .into_iter()
                    .collect(),
                strong: false,
                bare: false,
                place: true,
                marks: Vec::new(),
                bulge: 0.0,
            });
        }
        // The section that is picked says what it is set to, and what
        // can be done to it, in a card over its bracket's name.
        let card: Option<Element<'_, Message>> = braced
            .iter()
            .find(|braced| self.selected == Some(plan.steps()[braced.at].id))
            .and_then(|braced| {
                let step = &plan.steps()[braced.at];
                let first = beside
                    .iter()
                    .copied()
                    .find(|at| (braced.at + 1..braced.end).contains(at))
                    // One with nothing in it: beside its own place.
                    .unwrap_or(braced.at);
                let last = beside
                    .iter()
                    .copied()
                    .rfind(|at| (braced.at + 1..braced.end).contains(at))
                    .unwrap_or(braced.at);
                let (origin, top, foot) = {
                    let places = places.borrow();
                    (
                        *places.get(&track::Key::Canvas)?,
                        *places.get(&track::Key::Side(id_at(first)))?,
                        *places.get(&track::Key::Side(id_at(last)))?,
                    )
                };
                let wide = CARD;
                let middle = (top.y + foot.y + foot.height) / 2.0 - origin.y;
                // Right of the innermost brace, where the names are.
                let left = origin.width - track::side_room(side.lanes, side.wide) - side.more
                    + track::BRACE;
                let section = timing.sections.iter().find(|s| s.id == step.id);
                let card = container(self.section_card(plan, step, section))
                    .width(wide)
                    .padding([8, 6])
                    .style(look::card(p, 0.0, 12.0));
                Some(
                    iced::widget::pin(card)
                        .x(left.max(0.0))
                        .y((middle - 52.0).max(0.0))
                        .into(),
                )
            });
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
                let rows: Vec<Rectangle> = note
                    .on
                    .iter()
                    .filter_map(|on| crate::notes::anchor(plan, *on))
                    .filter_map(|id| places.get(&track::Key::Side(id)).copied())
                    .collect();
                let Some(first) = rows.iter().max_by(|a, b| a.y.total_cmp(&b.y)) else {
                    return Vec::new();
                };
                // From the card's top edge, near its mark, to where each
                // mark is: just left of the time at the row's end.
                let card = iced::Point::new(
                    (origin.width - crate::notes::CARD - 4.0).max(0.0) + 14.0,
                    first.y + first.height - origin.y + 2.0,
                );
                rows.iter()
                    .map(|row| {
                        (
                            card,
                            iced::Point::new(
                                row.x - origin.x - 44.0,
                                row.center_y() - origin.y + 7.0,
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let picture = track::Picture {
            stops,
            loops: drawn_loops,
            orders: drawn_orders,
            still: self.motion.still,
            hot: can,
            brackets,
            threads,
            select: (tree && can).then_some(p.primary),
            rows: whole,
            filling: self.filling.is_some(),
        };
        // Leaving the list means letting go would move nothing.
        let mut layers = stack![rows, track::over(picture, &places, Message::Marquee)].width(Fill);
        if let Some(card) = card {
            layers = layers.push(card);
        }
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
                .filter_map(|id| places.get(&track::Key::Side(id)).copied())
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
        // Steps picked together can be made a section: a mark beside the
        // last of them does it.
        let last_picked = self
            .picked
            .iter()
            .filter_map(|id| plan.position(*id))
            .max()
            // A bracket's mark goes by the last of what is in it.
            .map(|last| (last..plan.group_end(last)).last().unwrap_or(last))
            .filter(|_| self.picked.len() > 1 && tree && can && moving.is_none());
        // A step has them at its end; a section's place has them over it.
        if let Some(last) = last_picked.filter(|last| plan.steps()[*last].is_section())
            && let Some((origin, row)) = {
                let places = places.borrow();
                places
                    .get(&track::Key::Canvas)
                    .copied()
                    .zip(places.get(&track::Key::Side(id_at(last))).copied())
            }
        {
            layers = layers.push(
                iced::widget::pin(self.picked_actions())
                    // Beside the place, level with it.
                    .x((row.x - origin.x - 70.0).max(0.0))
                    .y((row.y - origin.y + 7.0).max(0.0)),
            );
        }
        mouse_area(scrolling(layers).id(ROWS))
            .on_exit(Message::LeftPlan)
            .into()
    }

    /// What can be done to steps picked together: make a section of them,
    /// or a note on them.
    fn picked_actions(&self) -> Element<'_, Message> {
        let p = self.palette;
        let make = hinted(
            p,
            button(text("{ + }").size(META).font(BOLD).color(p.primary))
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
    /// one the service is at, so the slide it is on is marked.
    fn step_parts<'a>(&self, step: &Step, on_stage: bool) -> Vec<Element<'a, Message>> {
        let p = self.palette;
        let Action::Show { resource } = &step.action else {
            return Vec::new();
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
                    .left(34.0)
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

    /// How the picked step fades, in a card under its name: in and out, a
    /// second less or more each press. Only for a step that shows
    /// something, where it can change.
    fn fade_card<'a>(&self, step: &Step) -> Option<Element<'a, Message>> {
        if !matches!(step.action, Action::Show { .. }) {
            return None;
        }
        let changes = self.can_edit()
            && (self.mode != crate::mode::Mode::Run || self.ahead(&self.engine.0.plan(), step.id));
        if !changes {
            return None;
        }
        let p = self.palette;
        Some(
            container(
                container(fade_steppers(p, step))
                    .padding([4, 5])
                    .style(look::card(p, 0.0, 10.0)),
            )
            .padding(iced::Padding::default().top(6.0))
            .into(),
        )
    }

    /// `row`, telling the app which half of it the pointer is over, so a
    /// held row can be dropped above or below it. While a drag moves, the
    /// pointer is a closed hand.
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

    fn step_row(
        &self,
        step: &Step,
        place: Place,
        issue: Option<&Issue>,
        noted: &[(lantern_engine::NoteId, usize)],
        picked_end: bool,
    ) -> Element<'_, Message> {
        let p = self.palette;
        // The current step is on stage unless something off the plan was
        // shown since.
        let shown = match &step.action {
            Action::Show { resource } => self
                .stage
                .program
                .as_ref()
                .is_some_and(|scene| scene.resource.id == resource.id),
            _ => false,
        };
        let tone = match place {
            Place::Current if shown => Tone::Live,
            _ if self.selected == Some(step.id) || self.picked.contains(&step.id) => Tone::Selected,
            _ => Tone::Plain,
        };
        // On stage, the row says how far into it the stage is.
        let progress = match (place, &self.stage.program) {
            (Place::Current, Some(scene)) if shown => progress(&scene.media, &scene.resource),
            _ => None,
        };
        let label_color = match (place, tone) {
            (Place::Done, _) => p.outline,
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
        } else if picked_end {
            self.picked_actions()
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
        let mut label = column![title];
        if self.editing(step.id, Part::Search) {
            label = label.push(self.search_field(step));
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
        let mut line = row![container(label).width(Fill).clip(true)]
            .spacing(8)
            .align_y(Alignment::Center);
        // Under the pointer, a row that can change says so: what would
        // replace it, and the way to take it out. The selected row has
        // all of that on its second line already.
        if self.pointed(step.id) {
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
        let line = match coming {
            Some(part) => line.push(hinted(
                p,
                look::ring(part, 16.0, p.tertiary),
                format!("Downloading: {:.0}%", part * 100.0),
                tooltip::Position::Top,
            )),
            None => line,
        };
        let line = match acting {
            true => line.push(self.step_actions(step)).push(end),
            false => line.push(end),
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
        // Just added, replaced or dropped here: it flashes once, so the
        // eye finds where it went.
        let fill = match self.motion.flash(step.id) {
            Some(faded) => crate::motion::blend(Palette::tint(p.primary, 0.5), fill, faded),
            None => fill,
        };
        // Picked, how it fades, under all of its line.
        let line: Element<'_, Message> = match self.fade_card(step).filter(|_| selected) {
            Some(card) => column![line, card].into(),
            None => line.into(),
        };
        let row = button(line)
            .width(Fill)
            .padding([10, 10])
            .on_press(Message::Click(Clicked::Step(step.id)))
            .style(look::row_filled(p, fill));
        row.into()
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

    /// How long the plan takes, large, over everything else in the
    /// pane: what is planned, the target to hold it against and how far
    /// off it is, when it starts and ends, and a bar with a part for each
    /// section, as wide as its time and as full as its steps make it.
    fn time_row<'a>(&self, plan: &Plan, timing: &Timing) -> Element<'a, Message> {
        let p = self.palette;
        let schedule = lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step));
        let total = schedule.total;
        // What the steps themselves add up to: sections' own times left
        // out, so it is what has been put in so far.
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
        let filled =
            lantern_engine::timing::schedule(&bare, &|step| self.loaded_length(step)).total;
        // What it is held against: the plan's target, or else what its
        // sections' times come to.
        let against = plan.target.or((total != filled).then_some(total));
        // A repeat with no number to it: at least this long.
        let planned = match schedule.open {
            true => format!("{}+", length::clock(filled)),
            false => length::clock(filled),
        };
        let mut top = row![
            look::icon(Icon::Clock, 16.0, p.primary),
            text(planned).size(22).font(BOLD).color(p.on_surface),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let target: Element<'a, Message> = match &self.target_typed {
            Some(typed) => text_input("45", typed)
                .id(TARGET_BOX)
                .on_input(Message::PlanTarget)
                .on_submit(Message::TargetDone)
                .size(META)
                .padding([3, 8])
                .width(64)
                .style(look::input(p))
                .into(),
            None => hinted(
                p,
                button(
                    text(match against {
                        Some(against) => format!("◎ {}", length::clock(against)),
                        None => "◎".to_string(),
                    })
                    .size(META)
                    .font(BOLD)
                    .color(if plan.target.is_some() {
                        p.primary
                    } else {
                        p.on_surface_variant
                    }),
                )
                .padding([3, 8])
                .on_press_maybe(self.can_edit().then_some(Message::EditTarget))
                .style(look::pill(p, Tone::Plain)),
                "How long the whole service is meant to take".to_string(),
                tooltip::Position::Bottom,
            ),
        };
        top = top.push(target).push(Space::new().width(Fill));
        if let Some(starts) = plan.starts {
            top = top.push(
                text(format!(
                    "{starts} → {}",
                    starts.plus(against.unwrap_or(total))
                ))
                .size(META)
                .color(p.on_surface_variant),
            );
        }
        // Under it: how far under or over, and how the plan itself moves.
        let mut under = row![].spacing(8).align_y(Alignment::Center);
        if let Some(against) = against {
            let (sign, off, colour) = match filled > against {
                true => ("↑", filled - against, p.error),
                false => ("↓", against - filled, p.tertiary),
            };
            if !off.is_zero() {
                let off = format!("{sign} {}", length::clock(off));
                under = under.push(
                    text(off.clone())
                        .size(META)
                        .color(colour)
                        .width(widgets::steady(&off, META)),
                );
            }
        }
        under = under
            .push(Space::new().width(Fill))
            .push(self.flow_buttons(None, plan.flow, false));
        // One part for each of the plan's own sections.
        let parts = timing
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
                let wide = section.budget.unwrap_or(planned).as_secs().clamp(1, 60_000) as u16;
                let (full, colour) = match section.budget {
                    Some(budget) if planned > budget => (1.0, p.tertiary),
                    Some(budget) if !budget.is_zero() => {
                        (planned.as_secs_f32() / budget.as_secs_f32(), p.primary)
                    }
                    _ => (1.0, p.primary),
                };
                container(bar(p, full, colour, 0.0))
                    .width(Length::FillPortion(wide))
                    .into()
            });
        column![top, under, row(parts).spacing(3)]
            .spacing(8)
            .padding(iced::Padding::default().left(8.0).right(8.0).bottom(8.0))
            .into()
    }

    pub(crate) fn flow_buttons<'a>(
        &self,
        of: Option<StepId>,
        flow: Flow,
        all: bool,
    ) -> Element<'a, Message> {
        let p = self.palette;
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
    /// waits. Planned is the plan's colour, live the stage's, past grey;
    /// under the step on the projector the line fills as it plays.
    /// A click under the dot joins the step to the next, or parts them.
    fn rail<'a>(
        &self,
        plan: &Plan,
        index: usize,
        place: Place,
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
        let planned = p.primary;
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
            Place::Later => track::Dot::Hollow(p.on_surface_variant),
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
            true => RAIL_TOP - RAIL_RING,
            false => RAIL_TOP,
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

    /// The place of a section with nothing in it: where its steps will
    /// be, dashed, with room on its right for its bracket. A press on it
    /// picks the section; steps carried over it land in it.
    fn empty_place<'a>(
        &self,
        section: StepId,
        lanes: Lanes<'_>,
        side: f32,
        places: &track::Places,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let pick = Message::Click(Clicked::Step(section));
        let inside = row![
            look::icon(Icon::Plus, 14.0, p.on_surface_variant),
            Space::new().width(Fill),
            look::icon(Icon::Fill, 14.0, Palette::tint(p.primary, 0.7)),
        ]
        .align_y(Alignment::Center)
        .padding([0, 10])
        .height(PLACE_TALL - 8.0);
        let slot = button(crate::drawn::dashed(
            container(inside)
                .width(Fill)
                .style(move |_| container::Style {
                    background: Some(Palette::tint(p.primary, 0.06).into()),
                    border: iced::Border {
                        radius: 9.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
            Palette::tint(p.primary, 0.6),
            9.0,
        ))
        .padding(0)
        .width(Fill)
        .on_press(pick.clone())
        .style(|_, _| button::Style::default());
        let mut beside = mouse_area(Space::new().width(side).height(Fill))
            .on_press(pick)
            .interaction(mouse::Interaction::Pointer);
        if self.can_edit() {
            beside = beside.on_right_press(Message::Rail(crate::rail::Act::Ring(
                crate::rail::RingOn::All(Some(section)),
            )));
        }
        let place = row![
            Space::new().width(lanes.curl + RAIL + lanes.drawn),
            container(slot).padding([4, 0]),
            track::seen(track::Key::Side(section), places, beside),
        ]
        .height(PLACE_TALL);
        track::seen(
            track::Key::Whole(section),
            places,
            self.draggable(section, place.into()),
        )
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

    /// A folded section's one row: its steps gathered into a stacked dot
    /// on the line, how many there are, and how long they take. Pressed,
    /// it opens, one over in a service too.
    fn folded_place<'a>(
        &self,
        plan: &Plan,
        step: &Step,
        over: bool,
        lanes: Lanes<'_>,
        side: f32,
        places: &track::Places,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let id = step.id;
        let at = plan.position(id).unwrap_or_default();
        let steps = (at + 1..plan.group_end(at))
            .filter(|row| !plan.steps()[*row].is_section())
            .count();
        let ink = match over {
            true => p.outline,
            false => p.on_surface,
        };
        let dot = column![
            Space::new().height(RAIL_TOP - 2.0),
            track::seen(
                track::Key::Dot(id),
                places,
                Space::new().width(RAIL_DOT).height(RAIL_DOT)
            ),
        ]
        .width(RAIL)
        .align_x(Alignment::Center);
        let mut words = row![
            text(match steps {
                1 => "1 step".to_string(),
                n => format!("{n} steps"),
            })
            .size(BODY)
            .color(ink),
            Space::new().width(Fill),
        ];
        if !over {
            words = words.push(
                text(length::clock(self.planned(plan, id)))
                    .size(META)
                    .color(p.on_surface_variant),
            );
        }
        let words = words.align_y(Alignment::Center).padding([0, 10]);
        let body = mouse_area(container(words).width(Fill).center_y(Fill))
            .interaction(mouse::Interaction::Pointer)
            .on_press(Message::Fold(id));
        // Its bracket: twice opens it, as twice folded it.
        let mut beside = mouse_area(Space::new().width(side).height(Fill))
            .on_double_click(Message::Fold(id))
            .interaction(mouse::Interaction::Pointer);
        if !over {
            beside = beside.on_press(Message::Click(Clicked::Step(id)));
        }
        let place = row![
            Space::new().width(lanes.curl),
            track::seen(track::Key::Row(id), places, dot),
            Space::new().width(lanes.drawn),
            body,
            track::seen(track::Key::Side(id), places, beside),
        ]
        .height(ROW_TALL);
        track::seen(
            track::Key::Whole(id),
            places,
            self.draggable(id, place.into()),
        )
    }

    /// What a picked section is set to and what can be done to it, one
    /// under another so it is no wider than its bracket's name was: its
    /// name, the time it is given (pressed, it is set), the time it
    /// takes, and filling it, renaming it and taking it out.
    fn section_card(
        &self,
        plan: &Plan,
        step: &Step,
        timing: Option<&SectionTiming>,
    ) -> Element<'_, Message> {
        let p = self.palette;
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
        fn act<'a>(
            p: Palette,
            mark: Element<'a, Message>,
            does: Message,
            says: &str,
        ) -> Element<'a, Message> {
            hinted(
                p,
                button(mark)
                    .padding([3, 7])
                    .on_press(does)
                    .style(look::row(p, Tone::Plain)),
                says.to_string(),
                tooltip::Position::Top,
            )
        }
        let mut acts = row![].spacing(2).align_y(Alignment::Center);
        if self.mode == crate::mode::Mode::Plan {
            acts = acts.push(act(
                p,
                look::icon(Icon::Fill, 14.0, p.primary).into(),
                Message::Fill(crate::fill_panel::FillEdit::Open(id)),
                "Fill it from the library",
            ));
        }
        if self.can_edit() {
            acts = acts
                .push(act(
                    p,
                    look::icon(Icon::Pencil, 13.0, p.primary).into(),
                    Message::EditStep(Some((id, Part::Name))),
                    "Rename",
                ))
                .push(act(
                    p,
                    look::icon(Icon::Note, 13.0, crate::notes::INK).into(),
                    Message::NoteFor(vec![lantern_engine::NoteOn::Step(id)]),
                    "A note on the section",
                ))
                .push(act(
                    p,
                    text("✕").size(META).color(p.on_surface_variant).into(),
                    Message::Remove(id),
                    "Take the section out; its steps stay",
                ));
        }
        let mut card = column![name, goal, by, sum].spacing(6);
        if let Some(ends) = ends {
            card = card.push(ends);
        }
        card.push(acts).into()
    }

    /// The room right of the step at `index` for the brackets of the
    /// sections it is in, a part for each lane, the innermost first. A
    /// press there picks that section, and held and moved carries it
    /// with all it holds; twice folds it away; a right click opens the
    /// ring of what holds for all of it.
    fn side<'a>(
        &self,
        plan: &Plan,
        index: usize,
        side: Side<'_>,
        places: &track::Places,
    ) -> Element<'a, Message> {
        let covering = |lane: Option<u8>| {
            side.braced
                .iter()
                .filter(|braced| (braced.at + 1..braced.end).contains(&index))
                .filter(|braced| lane.is_none_or(|lane| braced.lane == lane))
                .min_by_key(|braced| braced.lane)
                .map(|braced| plan.steps()[braced.at].id)
        };
        let part = |wide: f32, section: Option<StepId>| -> Element<'a, Message> {
            let part = Space::new().width(wide).height(Fill);
            let Some(section) = section else {
                return part.into();
            };
            let mut area = mouse_area(part)
                .on_press(Message::Grab(section))
                .on_double_click(Message::Fold(section))
                .interaction(mouse::Interaction::Pointer);
            if self.can_edit() {
                area = area.on_right_press(Message::Rail(crate::rail::Act::Ring(
                    crate::rail::RingOn::All(Some(section)),
                )));
            }
            // Beside the first and the last of its steps, the head and the
            // foot of its bracket: held and pulled, the head takes in what
            // comes before or lets go of its first rows, the foot takes in
            // what comes after or lets go of its last.
            let (first, last) = side
                .braced
                .iter()
                .find(|braced| plan.steps()[braced.at].id == section)
                .map(|braced| {
                    let mut steps =
                        (braced.at + 1..braced.end).filter(|row| !plan.steps()[*row].is_section());
                    let first = steps.next();
                    (first, steps.next_back().or(first))
                })
                .unwrap_or((None, None));
            let pulls = self.can_edit() && self.mode == crate::mode::Mode::Plan;
            let handle = |pressed: Message| {
                mouse_area(Space::new().width(wide).height(FOOT))
                    .on_press(pressed)
                    .interaction(mouse::Interaction::ResizingVertically)
            };
            let mut parts = column![];
            if pulls && first == Some(index) {
                parts = parts.push(handle(Message::PressHead(section)));
            }
            parts = parts.push(area);
            if pulls && last == Some(index) {
                parts = parts.push(handle(Message::PressFoot(section)));
            }
            let area: Element<'a, Message> = parts.into();
            // What is under the pointer here is this row's step, as on
            // the rest of the row: where something carried would land
            // goes by rows, and must not flip to the section's start as
            // the pointer crosses into its bracket's room.
            self.draggable(plan.steps()[index].id, area)
        };
        // The innermost brace and the names beside it: the section this
        // step is most nearly in. Then a brace for each one further out,
        // and the room right of those, which is the outermost's.
        let mut parts: Vec<Element<'a, Message>> = Vec::new();
        if side.lanes > 0 {
            parts.push(part(
                track::BRACE + track::LABEL + side.more,
                covering(None),
            ));
        }
        for lane in 1..side.lanes {
            parts.push(part(track::BRACE, covering(Some(lane))));
        }
        if side.wide {
            let outermost = side
                .lanes
                .checked_sub(1)
                .and_then(|lane| covering(Some(lane)));
            parts.push(part(track::LABEL, outermost));
        }
        track::seen(
            track::Key::Side(plan.steps()[index].id),
            places,
            row(parts)
                .width(track::side_room(side.lanes, side.wide) + side.more)
                .height(Fill),
        )
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
        let estimate = lantern_engine::timing::estimate(step, self.loaded_length(step));
        match estimate.basis {
            Basis::Guess if estimate.length.is_zero() => String::new(),
            Basis::Guess => length::rough(estimate.length),
            Basis::Set | Basis::Known => length::clock(estimate.length),
        }
    }

    fn saved_plans<'a>(&self, entries: &'a [Entry]) -> Element<'a, Message> {
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
                list = list.push(saved_plan(p, entry, when(entry), picked(entry)));
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
            list = list.push(saved_plan(p, entry, when(entry), picked(entry)));
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
        if let Some((typed, found)) = &self.importing {
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
            import = import.push(
                container(text(hint).size(META).color(p.on_surface_variant)).padding([4, 10]),
            );
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

/// How long `step` fades in and out, each a second less or more with −
/// and +.
pub(crate) fn fade_steppers<'a>(p: Palette, step: &Step) -> Element<'a, Message> {
    let id = step.id;
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
            hinted(p, ramp, name.to_string(), tooltip::Position::Left),
            small("−", now.map(|_| Message::SetFade(id, end, -1))),
            container(text(shown).size(META).font(BOLD).color(p.on_surface)).center_x(28),
            small(
                "+",
                (now != Some(FadeTime::LONGEST)).then_some(Message::SetFade(id, end, 1))
            ),
        ]
        .spacing(3)
        .align_y(Alignment::Center)
    };
    column![
        one("Fades in", FadeEnd::In, step.fade.fade_in),
        one("Fades out", FadeEnd::Out, step.fade.fade_out),
    ]
    .spacing(4)
    .into()
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
fn saved_plan(p: Palette, entry: &Entry, when: String, picked: bool) -> Element<'_, Message> {
    let (title, detail, color, template) = match &entry.summary {
        Ok(summary) => (
            summary.title.clone(),
            format!("{} · {when}", count(summary.steps, "step")),
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

/// The rows of the section something held would be a part of, with a
/// line down their side, `indent` in, `lit` of the way to lit.
fn hosted<'a>(
    p: Palette,
    rows: iced::widget::Column<'a, Message>,
    indent: f32,
    lit: f32,
) -> Element<'a, Message> {
    let line = container(
        container(Space::new().width(3).height(Fill)).style(move |_| container::Style {
            background: Some(p.primary.scale_alpha(lit).into()),
            border: iced::Border {
                radius: 2.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        }),
    )
    .padding(iced::Padding::default().left(indent));
    stack![rows, line].into()
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
