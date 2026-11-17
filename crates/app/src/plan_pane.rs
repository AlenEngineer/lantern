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
use iced::{Alignment, Element, Fill, Length, mouse};
use lantern_engine::{
    Action, Basis, Flow, Media, Pace, Phase, Plan, Query, Repeat, Resource, SectionTiming, Step,
    StepId, TimeOfDay, Timing,
};

use crate::dashboard::{NAME_BOX, level_tone, op, pill, scrolling, single_line, unit, wide};
use crate::drag::{Drag, Half};
use crate::keys::{self, Op};
use crate::length;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL_SMALL, Tone};
use crate::palette::Palette;
use crate::plan_edit::{NewStep, Part, goes_on_when, insertion_mark};
use crate::plans::{self, Entry, Notice, Switch, file_name};
use crate::ready::Issue;
use crate::{App, Clicked, Message};

/// The id of the pane's rows, to scroll them.
pub const ROWS: &str = "plan-rows";

/// How far a section's steps sit in from its header.
const INDENT: f32 = 12.0;

/// The field the plan's target time is typed in.
pub(crate) const TARGET_BOX: &str = "plan-target";

/// The rail beside the steps: how wide its column is, its dots, its
/// line, and how far down a row its dot sits, level with the step's name.
const RAIL: f32 = 20.0;
const RAIL_DOT: f32 = 10.0;
const RAIL_LINE: f32 = 2.0;
const RAIL_TOP: f32 = 15.0;

/// About how tall a step's row is, and each line under it, and the gap a
/// drag opens: for what is cut to a height on its way open or closed. A
/// little over is right; under would cut a row that should show whole.
const ROW_TALL: f32 = 44.0;
const PART_TALL: f32 = 22.0;
const GAP_TALL: f32 = 48.0;

/// A section's steps, cut to as much of their height as is in sight.
fn cut<'a>(
    (steps, tall, open): (iced::widget::Column<'a, Message>, f32, f32),
) -> Element<'a, Message> {
    container(steps).max_height(tall * open).clip(true).into()
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
            _ => button(
                container(
                    single_line(plan.title.clone())
                        .size(HEADING)
                        .font(BOLD)
                        .color(p.primary),
                )
                .clip(true),
            )
            .padding([4, 8])
            .on_press(Message::EditPlanName)
            .style(look::row(p, Tone::Plain))
            .width(Fill)
            .into(),
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
        // Everything about the plan that isn't its steps.
        if !running {
            header = header.push(
                tooltip(
                    button(text("⋯").size(HEADING).color(p.on_surface_variant))
                        .padding([2, 10])
                        .on_press(Message::PlanSettings)
                        .style(look::row(p, Tone::Plain)),
                    container(text("This plan: name, look, template, copy, delete").size(META))
                        .padding([4, 8])
                        .style(look::tip(p)),
                    tooltip::Position::Bottom,
                )
                .delay(look::TIP_DELAY),
            );
        }
        if self.plans.is_dirty(plan) {
            header = header.push(crate::dashboard::hinted(
                p,
                pill(p, "Save", Tone::Selected, PILL_SMALL, Some(op(Op::Save))),
                keys::with_key("Save the plan", Op::Save),
                tooltip::Position::Bottom,
            ));
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
        let closing_gap = |left: f32| Space::new().height(GAP_TALL * left);
        let gap = |indent: f32| {
            let gap = container(drop_gap(
                p,
                moving
                    .map(|d| d.label(plan))
                    .or_else(|| held.map(|held| held.file.title.clone()))
                    .unwrap_or_default(),
            ))
            .padding(iced::Padding::default().left(indent));
            match opening {
                Some(open) => gap.max_height(GAP_TALL * open).clip(true),
                None => gap,
            }
        };
        // The pink line is there only while a search has a result for
        // Ctrl+Enter to add.
        let adds = self.search.first().is_some();
        let mut rows = column![].spacing(2);
        // While planning, the pane is a tree: the open plan, its sections
        // and their steps, and under them the other plans.
        let tree = self.mode == crate::mode::Mode::Plan;
        let running = self.mode == crate::mode::Mode::Run;
        if tree {
            rows = rows.push(self.time_row(plan, timing));
        } else if running {
            rows = rows.push(self.run_time_row(plan, timing));
        }
        if !tree && let Some(card) = self.repeat_card(plan) {
            // Away from Plan there is no side panel for it: the choices of
            // how often something comes sit over the plan.
            rows = rows.push(container(card).padding(8));
        }
        // In a service with a start time, a section still to come says
        // when it is due.
        let due = plan.starts.filter(|_| running).map(|starts| {
            let schedule = lantern_engine::timing::schedule(plan, &|step| self.loaded_length(step));
            move |id| schedule.start_of(id).map(|after| starts.plus(after))
        });
        let mut in_section = false;
        // Up to where rows are folded away with the section that holds
        // them, and whether that section is the one closing or opening
        // now: then they are still drawn, under the cut.
        let mut hidden_to = 0;
        let mut hidden_by_moving = false;
        // Up to where the section that is closing or opening reaches.
        let mut folding_to = 0;
        let mut rest = 0;
        // The steps of a section that is closing or opening, kept apart
        // to be cut to the height they have so far: with how tall they
        // are in all, and how much of that is in sight.
        let mut folding: Option<(iced::widget::Column<'_, Message>, f32, f32)> = None;
        for (index, step) in plan.steps().iter().enumerate() {
            if carried.contains(&step.id) {
                continue;
            }
            if landing == Some(rest) {
                rows = rows.push(gap(gap_indent(moving, in_section)));
            }
            if let Some((_, left)) = closing.filter(|(at, _)| *at == rest) {
                rows = rows.push(closing_gap(left));
            }
            rest += 1;
            let selected = self.selected == Some(step.id) && moving.is_none();
            let inward = INDENT * f32::from(plan.level(index));
            let hidden = index < hidden_to && !(hidden_by_moving && index < folding_to);
            if step.is_section() {
                // The section that was closing or opening ends here.
                if index >= folding_to
                    && let Some(steps) = folding.take()
                {
                    rows = rows.push(cut(steps));
                }
                in_section = true;
                // Folded away with the section it is in.
                if hidden && !selected {
                    continue;
                }
                let section = timing.sections.iter().find(|s| s.id == step.id);
                // In a service, a section that is over folds away to its
                // one line: what it was, and how long it took.
                let over = running
                    && section.is_some_and(|section| matches!(section.phase, Phase::Done { .. }));
                let folded = over || self.folded.contains(&step.id);
                if folded && index >= hidden_to {
                    hidden_to = plan.group_end(index);
                    hidden_by_moving = false;
                }
                // Its arrow folds its steps away, and opens them again.
                let id = step.id;
                // A quiet heading has no arrow; a folded one says so, and
                // under the pointer or selected it can be folded.
                // One that is over in a service is folded as a matter of
                // course: its tick says so, with no arrow.
                let reach = (folded && !over)
                    || selected
                    || self.hovered.is_some_and(|(pointed, _)| pointed == id);
                let arrow = button(text(if folded { "▸" } else { "▾" }).size(META).color(
                    if reach {
                        p.on_surface_variant
                    } else {
                        iced::Color::TRANSPARENT
                    },
                ))
                .padding([4, 3])
                .on_press(Message::Fold(id))
                .style(look::row(p, Tone::Plain));
                // Between two sections: whether the one before hands
                // over by itself or waits.
                let before = plan.steps()[..index]
                    .iter()
                    .rposition(|step| !step.is_section());
                if let Some(before) = before
                    && !plan.goes_on(before)
                    && folding.is_none()
                    && !hidden
                {
                    rows = rows.push(
                        container(look::icon(Icon::Pause, 12.0, p.on_surface_variant))
                            .padding(iced::Padding::default().left(inward + INDENT + 4.0)),
                    );
                }
                let heading = container(
                    row![
                        arrow,
                        self.draggable(
                            step.id,
                            self.section_row(
                                plan,
                                step,
                                section,
                                due.as_ref().and_then(|due| due(step.id))
                            )
                        )
                    ]
                    .align_y(Alignment::Center),
                )
                .padding(iced::Padding::default().left(inward));
                // A section inside the one that is closing or opening
                // goes under the cut with its steps.
                match folding.take() {
                    Some((steps, so_far, open)) => {
                        folding = Some((steps.push(heading), so_far + ROW_TALL, open));
                    }
                    None => rows = rows.push(heading),
                }
                if selected && adds {
                    rows = rows.push(insertion_mark(p, inward + INDENT));
                }
                if folding.is_none()
                    && let Some(open) = self.motion.open(step.id)
                {
                    folding = Some((column![].spacing(2), 0.0, open));
                    folding_to = plan.group_end(index);
                    hidden_by_moving = hidden_to == folding_to;
                }
                continue;
            }
            // Folded away with its section, unless it is the step the
            // service is at or the one being worked on. On its way there
            // it is still drawn, under the cut.
            if hidden && Some(step.id) != current && !selected {
                continue;
            }
            let place = if Some(step.id) == current {
                Place::Current
            } else if Some(step.id) == next {
                Place::Next
            } else if at.is_some_and(|at| index < at) {
                Place::Done
            } else {
                Place::Later
            };
            let issue = issues.iter().find(|i| i.step == step.id);
            let mut row = column![self.draggable(step.id, self.step_row(step, place, issue))];
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
            let row = container(row![self.rail(plan, index, place), row].height(Length::Shrink))
                .padding(iced::Padding::default().left(inward));
            // A step that stays in sight when its section is folded (the
            // one the service is at, the one being worked on) stays put
            // while the others close around it.
            let stays = Some(step.id) == current || selected;
            match folding.take() {
                Some((steps, so_far, open)) if stays => {
                    rows = rows.push(cut((steps, so_far, open))).push(row);
                    folding = Some((column![].spacing(2), 0.0, open));
                }
                Some((steps, so_far, open)) => {
                    folding = Some((steps.push(row), so_far + tall, open));
                }
                None => rows = rows.push(row),
            }
        }
        if let Some(steps) = folding.take() {
            rows = rows.push(cut(steps));
        }
        if landing == Some(rest) {
            rows = rows.push(gap(gap_indent(moving, in_section)));
        }
        if let Some((_, left)) = closing.filter(|(at, _)| *at == rest) {
            rows = rows.push(closing_gap(left));
        }
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
            rows = rows.push(list_action(
                p,
                if self.adding.is_some() { "−" } else { "+" },
                String::new(),
                Message::ToggleAdd,
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
        // Leaving the list means letting go would move nothing.
        mouse_area(scrolling(rows).id(ROWS))
            .on_exit(Message::LeftPlan)
            .into()
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

    /// A section's header: its name and its time in words ("12 of 15 min
    /// planned", "6:40 of 15 min") over a bar, or a button to set one.
    /// Clicking it selects it, to rename it or change its time.
    fn section_row(
        &self,
        plan: &Plan,
        step: &Step,
        timing: Option<&SectionTiming>,
        due: Option<TimeOfDay>,
    ) -> Element<'_, Message> {
        let p = self.palette;
        let id = step.id;
        let budget = match step.action {
            Action::Section { budget, .. } => budget,
            _ => None,
        };
        let phase = timing.map_or(
            Phase::Coming {
                planned: self.planned(plan, id),
            },
            |t| t.phase,
        );
        let selected =
            self.selected == Some(id) && !self.drag.as_ref().is_some_and(|drag| drag.moving);
        let running = self.mode == crate::mode::Mode::Run;
        // Over, in a service: a tick, its name and how long it took, in
        // the colour of what is past.
        if let (true, Phase::Done { took }) = (running, &phase) {
            return button(
                row![
                    look::icon(Icon::Check, 14.0, p.outline),
                    container(
                        single_line(step.label.clone())
                            .size(BODY)
                            .font(BOLD)
                            .color(p.outline)
                    )
                    .width(Fill)
                    .clip(true),
                    text(took.map(length::clock).unwrap_or_default())
                        .size(META)
                        .color(p.outline),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding([6, 8])
            .on_press(Message::Click(Clicked::Step(id)))
            .style(look::row(p, Tone::Plain))
            .into();
        }
        // The name is edited where it's read. The section the service is
        // in has the stage's colour.
        let editing_name = self.editing(id, Part::Name);
        let on_now = running && matches!(phase, Phase::Running { .. });
        let title: Element<'_, Message> = if editing_name {
            self.name_field(step)
        } else {
            single_line(step.label.clone())
                .size(BODY)
                .font(BOLD)
                .color(if on_now { p.tertiary } else { p.primary })
                .into()
        };
        // One line: its name, how full its time is as a bar, the same in
        // numbers, how it moves, and the way to rehearse it.
        let minutes = |length: Duration| (length.as_secs() + 30) / 60;
        let part = |of: Duration, whole: Duration| of.as_secs_f32() / whole.as_secs_f32();
        let quiet = p.on_surface_variant;
        let (numbers, full, colour) = match (&phase, budget.filter(|budget| !budget.is_zero())) {
            // In a service: when it is due, and how long it has.
            (Phase::Coming { .. }, Some(budget)) if running => (
                match due {
                    Some(due) => format!("{due} · {}", length::clock(budget)),
                    None => length::clock(budget),
                },
                None,
                quiet,
            ),
            (Phase::Coming { planned }, Some(budget)) => (
                format!("{}/{}", minutes(*planned), minutes(budget)),
                Some(part(*planned, budget)),
                if *planned > budget { p.tertiary } else { quiet },
            ),
            (Phase::Coming { planned }, None) => (length::clock(*planned), None, quiet),
            (Phase::Running { elapsed, expected }, Some(budget)) => (
                format!("{} / {}", length::clock(*elapsed), length::clock(budget)),
                Some(part(*elapsed, budget)),
                if *expected > budget {
                    p.error
                } else {
                    p.tertiary
                },
            ),
            (Phase::Running { elapsed, .. }, None) => (length::clock(*elapsed), None, p.tertiary),
            (Phase::Done { took }, _) => {
                (took.map(length::clock).unwrap_or_default(), None, p.outline)
            }
        };
        let fill = match &phase {
            Phase::Running { .. } => p.tertiary,
            Phase::Coming { planned } if budget.is_some_and(|budget| *planned > budget) => {
                p.tertiary
            }
            _ => p.primary,
        };
        // Selected, its row has every choice of how it moves: the name
        // gives way so they all fit, and the bar is left out.
        let mut line = row![match editing_name || selected {
            true => container(title).width(Fill).clip(true),
            false => container(title),
        }]
        .spacing(6)
        .align_y(Alignment::Center);
        if !editing_name && !selected {
            line = line.push(match full {
                Some(full) => container(bar(p, full, fill, 0.0)).width(Fill),
                None => container(Space::new()).width(Fill),
            });
        }
        let set_time = Message::EditStep(Some((id, Part::Length)));
        if self.editing(id, Part::Length) {
            // The time is edited where it's read.
            line = line.push(self.section_time_field(id, budget));
        } else if selected {
            // On the selected section a click on its time changes it; one
            // without a time offers to set one.
            let words = match budget {
                Some(_) => numbers,
                None => format!("◎ {numbers}"),
            };
            line = line.push(
                button(text(words).size(META).color(colour))
                    .padding([2, 6])
                    .on_press(set_time)
                    .style(look::pill(p, Tone::Plain)),
            );
        } else {
            line = line.push(text(numbers).size(META).color(colour));
        }
        let goes = step.advance == Some(true);
        line = line.push(self.flow_buttons(Some(id), goes, step.flow, selected));
        // Under the pointer, a heading can be taken out like a step.
        if self.pointed(id) {
            line = line.push(self.pointed_actions(id));
        }
        let mut content = column![line].spacing(6);
        // What can be done to the selected section, on a line of its
        // own: nothing on the first line moves as the pointer comes.
        if selected {
            content = content.push(row![Space::new().width(Fill), self.step_actions(step)]);
        }
        let tone = if self.selected == Some(id) {
            Tone::Selected
        } else {
            Tone::Plain
        };
        button(content)
            .width(Fill)
            .padding([6, 8])
            .on_press(Message::Click(Clicked::Step(id)))
            .style(look::row(p, tone))
            .into()
    }

    fn step_row(&self, step: &Step, place: Place, issue: Option<&Issue>) -> Element<'_, Message> {
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
            _ if self.selected == Some(step.id) => Tone::Selected,
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
        let end: Element<'_, Message> = if self.editing(step.id, Part::Length) {
            self.step_length_field(step)
        } else if selected && has_length {
            let shown = if length.is_empty() {
                "min".to_string()
            } else {
                length.clone()
            };
            // Filled like a button, so it reads as something to click.
            button(text(shown).size(META).color(p.on_surface))
                .padding([2, 6])
                .on_press(Message::EditStep(Some((step.id, Part::Length))))
                .style(look::pill(p, Tone::Plain))
                .into()
        } else {
            // What there is to say, else the length; a step that goes on
            // by itself is marked.
            // What there is to say, else the length. Whether it goes on
            // by itself is the rail's to show.
            let words = said.clone().unwrap_or_else(|| length.clone());
            let color = if said.is_some() {
                said_color
            } else {
                p.on_surface_variant
            };
            text(words)
                .size(META)
                .color(color)
                .wrapping(iced::widget::text::Wrapping::None)
                .into()
        };
        let mut label = column![title];
        if self.editing(step.id, Part::Search) {
            label = label.push(self.search_field(step));
        } else if selected && let Some(said) = &said {
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
            Some(part) => line.push(crate::dashboard::hinted(
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
        let row = button(line)
            .width(Fill)
            .padding([10, 10])
            .on_press(Message::Click(Clicked::Step(step.id)))
            .style(look::row_filled(p, fill));
        // The bar lies in the row's bottom padding, so a step going on
        // stage doesn't make its row taller and move the ones under it.
        match progress {
            Some((_, done)) => stack![
                row,
                container(bar(p, done, p.tertiary, 24.0))
                    .align_bottom(Fill)
                    .padding(iced::Padding::default().left(10.0).right(10.0).bottom(3.0)),
            ]
            .into(),
            None => row.into(),
        }
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
                .color(p.on_surface),
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
                top = top.push(
                    text(format!(
                        "{sign} {}",
                        length::clock(Duration::from_secs(late.unsigned_abs()))
                    ))
                    .size(META)
                    .font(BOLD)
                    .color(colour),
                );
            }
            top = top.push(
                text(format!(
                    "→ {}",
                    length::clock_of_day(length::now_of_day() + left)
                ))
                .size(META)
                .color(p.on_surface_variant),
            );
        }
        let parts = timing
            .sections
            .iter()
            .filter(|section| plan.get(section.id).and_then(Step::depth) == Some(0))
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
            None => crate::dashboard::hinted(
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
                under = under.push(
                    text(format!("{sign} {}", length::clock(off)))
                        .size(META)
                        .color(colour),
                );
            }
        }
        under = under.push(Space::new().width(Fill)).push(self.flow_buttons(
            None,
            plan.advance,
            plan.flow,
            false,
        ));
        // One part for each of the plan's own sections.
        let parts = timing
            .sections
            .iter()
            .filter(|section| plan.get(section.id).and_then(Step::depth) == Some(0))
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

    /// How a section or the plan (`None`) moves, as its buttons: its
    /// steps go on by themselves or wait, it comes again, its steps come
    /// in a drawn order. What is on is always in sight; with `all`, what
    /// is off is too, to be turned on.
    pub(crate) fn flow_buttons<'a>(
        &self,
        of: Option<StepId>,
        goes: bool,
        flow: Flow,
        all: bool,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let can = self.can_edit();
        let glyph = |icon: Icon, on: bool| {
            look::icon(
                icon,
                14.0,
                if on { p.primary } else { p.on_surface_variant },
            )
        };
        let small = |content: Element<'a, Message>, on: bool, about: &str, message: Message| {
            crate::dashboard::hinted(
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
                match of {
                    Some(id) => Message::SetAdvance(id, !goes),
                    None => Message::PlanAdvance(!goes),
                },
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
            buttons = buttons.push(small(
                content.into(),
                repeats,
                "How many times it comes",
                Message::RepeatPanel(of),
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
                Message::SetShuffle(of, !flow.shuffle),
            ));
        }
        // Rehearse it in the preview; while it is, the way to stop.
        let rehearsing = self.rehearsal.as_ref().is_some_and(|r| r.of == of);
        if self.mode == crate::mode::Mode::Plan {
            buttons = buttons.push(crate::dashboard::hinted(
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

    /// How often a step, a section or the plan comes, as a card of its
    /// own: its name, a switch with four places (once, a number of times,
    /// until you go elsewhere, until a time of day), and under it what
    /// the place in use needs: a number to step, with what that comes to,
    /// or a time to type. `None` with nothing's choices open.
    pub(crate) fn repeat_card<'a>(&self, plan: &Plan) -> Option<Element<'a, Message>> {
        let of = self.repeat_panel?;
        let p = self.palette;
        let loaded = |step: &Step| self.loaded_length(step);
        // Its name, how often it comes now, and how long once round is.
        let (name, repeat, once) = match of {
            Some(id) => {
                let step = plan.get(id)?;
                let once = match plan.part(id) {
                    Some(mut part) => {
                        part.flow.repeat = Repeat::Once;
                        lantern_engine::timing::schedule(&part, &loaded).total
                    }
                    None => lantern_engine::timing::estimate(step, loaded(step)).length,
                };
                (step.label.clone(), step.flow.repeat, once)
            }
            None => {
                let mut whole = plan.clone();
                whole.flow.repeat = Repeat::Once;
                (
                    plan.title.clone(),
                    plan.flow.repeat,
                    lantern_engine::timing::schedule(&whole, &loaded).total,
                )
            }
        };
        let times = match repeat {
            Repeat::Times(times) => times.max(2),
            _ => 2,
        };
        // Until a time starts at the plan's start, or else the next half
        // hour: a loop before the service is what it is mostly for.
        let then = plan.starts.or_else(|| {
            let minutes = length::now_of_day().as_secs() / 60;
            let next = (minutes / 30 + 1) * 30 % (24 * 60);
            TimeOfDay::parse(&format!("{}:{:02}", next / 60, next % 60))
        });
        let set = |repeat| Message::SetRepeat(of, repeat);
        let place =
            |content: Element<'a, Message>, on: bool, about: &str, message: Option<Message>| {
                crate::dashboard::hinted(
                    p,
                    button(container(content).center_x(Fill))
                        .width(Fill)
                        .padding([6, 0])
                        .on_press_maybe(message)
                        .style(look::pill(p, if on { Tone::Solid } else { Tone::Plain })),
                    about.to_string(),
                    tooltip::Position::Bottom,
                )
            };
        let ink = |on: bool| if on { p.on_primary } else { p.on_surface };
        let word = |label: &'static str, on: bool| -> Element<'a, Message> {
            text(label).size(BODY).font(BOLD).color(ink(on)).into()
        };
        let (once_on, times_on, ever_on, until_on) = (
            repeat == Repeat::Once,
            matches!(repeat, Repeat::Times(_)),
            repeat == Repeat::Forever,
            matches!(repeat, Repeat::Until(_)),
        );
        let places = container(
            row![
                place(word("1", once_on), once_on, "Once", Some(set(Repeat::Once))),
                place(
                    word("×", times_on),
                    times_on,
                    "A number of times",
                    Some(set(Repeat::Times(times))),
                ),
                place(
                    word("∞", ever_on),
                    ever_on,
                    "Until you go somewhere else",
                    Some(set(Repeat::Forever)),
                ),
                place(
                    look::icon(Icon::Clock, 16.0, ink(until_on)).into(),
                    until_on,
                    "Until a time of day",
                    then.map(|then| set(Repeat::Until(then))),
                ),
            ]
            .spacing(2),
        )
        .padding(3)
        .style(move |_| container::Style {
            background: Some(p.surface_container_highest.into()),
            border: iced::Border {
                radius: 16.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });
        let mut card = column![
            row![
                look::icon(Icon::Repeat, 16.0, p.primary),
                container(
                    text(name)
                        .size(BODY)
                        .font(BOLD)
                        .color(p.primary)
                        .wrapping(text::Wrapping::None)
                )
                .width(Fill)
                .clip(true),
                button(text("✕").size(META).color(p.on_surface_variant))
                    .padding([2, 6])
                    .on_press(Message::RepeatPanel(of))
                    .style(look::row(p, Tone::Plain)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            places,
        ]
        .spacing(12);
        let round = |label: &'static str, message: Message| {
            button(
                container(text(label).size(HEADING).color(p.on_surface))
                    .center_x(34)
                    .center_y(34),
            )
            .padding(0)
            .on_press(message)
            .style(look::pill(p, Tone::Plain))
        };
        match repeat {
            Repeat::Times(_) => {
                let fewer = match times {
                    0..=2 => Repeat::Once,
                    times => Repeat::Times(times - 1),
                };
                card = card
                    .push(
                        container(
                            row![
                                round("−", set(fewer)),
                                text(format!("×{times}"))
                                    .size(26)
                                    .font(BOLD)
                                    .width(64)
                                    .align_x(iced::alignment::Horizontal::Center),
                                round("+", set(Repeat::Times(times.saturating_add(1)))),
                            ]
                            .spacing(14)
                            .align_y(Alignment::Center),
                        )
                        .center_x(Fill),
                    )
                    .push(
                        container(
                            row![
                                look::icon(Icon::Clock, 14.0, p.on_surface_variant),
                                text(format!(
                                    "{} × {times} = {}",
                                    length::clock(once),
                                    length::clock(once * u32::from(times))
                                ))
                                .size(META)
                                .color(p.on_surface_variant),
                            ]
                            .spacing(6)
                            .align_y(Alignment::Center),
                        )
                        .center_x(Fill),
                    );
            }
            Repeat::Until(time) => {
                let shown = time.to_string();
                card = card.push(
                    container(
                        text_input(&shown, &self.repeat_until)
                            .on_input(Message::RepeatUntil)
                            .on_submit(Message::RepeatUntilSet)
                            .size(22)
                            .padding([6, 10])
                            .width(96)
                            .align_x(iced::alignment::Horizontal::Center)
                            .style(look::input(p)),
                    )
                    .center_x(Fill),
                );
            }
            Repeat::Once | Repeat::Forever => {}
        }
        Some(card.into())
    }

    /// The rail beside the step at `index`: its dot, the line down to
    /// it from the step before when that one goes on by itself, and the
    /// line on to the next when this one does. A gap is a step that
    /// waits. Planned is the plan's colour, live the stage's, past grey;
    /// under the step on the projector the line fills as it plays.
    /// A click under the dot joins the step to the next, or parts them.
    fn rail<'a>(&self, plan: &Plan, index: usize, place: Place) -> Element<'a, Message> {
        let p = self.palette;
        let steps = plan.steps();
        let step = &steps[index];
        let goes = plan.goes_on(index);
        let before = steps[..index].iter().rposition(|step| !step.is_section());
        let after = steps[index + 1..].iter().any(|step| !step.is_section());
        let joined_above = before.is_some_and(|before| plan.goes_on(before));
        let joined_below = goes && after;
        let (planned, past) = (p.primary, p.outline);
        // A rehearsal is the plan's colour: nothing of it is live.
        let rehearsed = self.rehearsed_part(step.id);
        let (place, live) = match rehearsed {
            Some(_) => (Place::Current, planned),
            None => (place, p.tertiary),
        };
        let faint = Palette::tint(live, 0.35);
        // How far the stage is into this step, when it is the one up.
        let played = match (place, &step.action, &self.stage.program) {
            (Place::Current, Action::Show { resource }, Some(scene))
                if scene.resource.id == resource.id =>
            {
                Some(progress(&scene.media, &scene.resource).map_or(0.0, |(_, done)| done))
            }
            _ => None,
        }
        .or(rehearsed);
        let line = |colour: Option<iced::Color>, height: Length| {
            container(Space::new().width(RAIL_LINE).height(height)).style(move |_| {
                container::Style {
                    background: colour.map(Into::into),
                    ..container::Style::default()
                }
            })
        };
        let above = line(
            joined_above.then_some(match place {
                Place::Done | Place::Current => past,
                Place::Next => faint,
                Place::Later => planned,
            }),
            Length::Fixed(RAIL_TOP),
        );
        let filled = |colour: iced::Color, ring: Option<f32>| {
            container(Space::new().width(RAIL_DOT).height(RAIL_DOT)).style(move |theme| {
                container::Style {
                    border: iced::Border {
                        radius: (RAIL_DOT / 2.0).into(),
                        ..iced::Border::default()
                    },
                    ..look::dot(colour, ring)(theme)
                }
            })
        };
        let hollow = |colour: iced::Color| {
            container(Space::new().width(RAIL_DOT).height(RAIL_DOT)).style(move |_| {
                container::Style {
                    border: iced::Border {
                        color: colour,
                        width: 2.0,
                        radius: (RAIL_DOT / 2.0).into(),
                    },
                    ..container::Style::default()
                }
            })
        };
        // "Next" is the service's word: while planning, a step is only
        // one that goes on or one that waits.
        let place = match place {
            Place::Next if self.mode != crate::mode::Mode::Run => Place::Later,
            place => place,
        };
        let dot = match place {
            Place::Current => filled(live, self.motion.ring().filter(|_| played.is_some())),
            Place::Next => hollow(live),
            Place::Done => filled(past, None),
            Place::Later if goes => filled(planned, None),
            Place::Later => hollow(p.on_surface_variant),
        };
        let below: Element<'a, Message> = match (joined_below, place, played) {
            (false, ..) => Space::new().height(Fill).into(),
            (true, Place::Current, Some(done)) => {
                let done = (done.clamp(0.0, 1.0) * 100.0).round() as u16;
                column![
                    line(Some(live), Length::FillPortion(done.max(1))),
                    line(Some(faint), Length::FillPortion((100 - done).max(1))),
                ]
                .into()
            }
            (true, Place::Current, None) => line(Some(faint), Fill).into(),
            (true, Place::Done, _) => line(Some(past), Fill).into(),
            (true, ..) => line(Some(planned), Fill).into(),
        };
        let below = container(below).width(RAIL).height(Fill).center_x(RAIL);
        // What is still ahead can be joined to the next step or parted
        // from it, where the plan can change.
        let ahead = matches!(place, Place::Next | Place::Later);
        let below: Element<'a, Message> =
            if ahead && after && self.can_edit() && goes_on_when(step).is_some() {
                tooltip(
                    mouse_area(below)
                        .on_press(Message::SetAdvance(step.id, !goes))
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
            } else {
                below.into()
            };
        let rail = column![above, dot, below]
            .width(RAIL)
            .height(Fill)
            .align_x(Alignment::Center);
        // Beside it, the line that curls back from the last step of
        // something that repeats to its first. Every row keeps its room,
        // so the dots stay in one line down the plan.
        let curl: Element<'a, Message> = match curl_at(plan, index) {
            Some(piece) => {
                let under: Element<'a, Message> = match piece {
                    look::Curl::Out => Space::new().height(Fill).into(),
                    _ => look::curl(look::Curl::Down, planned).into(),
                };
                column![look::curl(piece, planned), under]
                    .width(look::CURL_WIDE)
                    .height(Fill)
                    .into()
            }
            None => Space::new().width(look::CURL_WIDE).into(),
        };
        // A width of its own: laid out after the step beside it, a row
        // that takes what it needs would be left none.
        row![curl, rail]
            .width(look::CURL_WIDE + RAIL)
            .height(Fill)
            .into()
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
        list = list.push(crate::dashboard::hinted(
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
    button(
        row![
            text(sign)
                .size(HEADING)
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
    let done = done.clamp(0.0, 1.0);
    let part = |portion: f32| (portion * 1000.0).round().max(1.0) as u16;
    let fill =
        |color: iced::Color| move |_: &iced::Theme| container::Style::default().background(color);
    let filled = container(Space::new())
        .height(3)
        .width(Length::FillPortion(part(done)));
    let rest = container(Space::new())
        .height(3)
        .width(Length::FillPortion(part(1.0 - done)));
    container(row![
        filled.style(fill(color)),
        rest.style(fill(p.surface_container_highest)),
    ])
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

/// A dragged step lands indented inside a section; a section never does.
/// The piece of the curl beside the step at `index`, when it is in
/// something that repeats: the nearest section around it that does, or
/// else the plan. `None` for a step in nothing that repeats, and for the
/// only step of something that does (its own mark says so).
fn curl_at(plan: &Plan, index: usize) -> Option<look::Curl> {
    let steps = plan.steps();
    let (start, end) = plan
        .within(index)
        .into_iter()
        .rev()
        .filter_map(|id| plan.position(id))
        .find(|at| steps[*at].flow.repeat != Repeat::Once)
        .map(|at| (at + 1, plan.group_end(at)))
        .or((plan.flow.repeat != Repeat::Once).then_some((0, steps.len())))?;
    let mut inside = (start..end).filter(|at| !steps[*at].is_section());
    let first = inside.next()?;
    let last = inside.next_back()?;
    Some(match index {
        at if at == first => look::Curl::Into,
        at if at == last => look::Curl::Out,
        _ => look::Curl::Past,
    })
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

fn gap_indent(moving: Option<&Drag>, in_section: bool) -> f32 {
    let step = moving.is_some_and(|drag| drag.carried.len() == 1 && !drag.is_section());
    if step && in_section { INDENT } else { 0.0 }
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
    fn the_curl_runs_from_the_last_step_of_what_repeats_to_its_first() {
        use look::Curl;
        let mut plan = Plan::new("Sabbath");
        let music = plan.section("Music", None);
        for label in ["A", "B", "C"] {
            plan.push(label, Action::Blank);
        }
        let sermon = plan.section("Sermon", None);
        plan.push("D", Action::Blank);
        // Nothing repeats: no curl.
        assert_eq!(curl_at(&plan, 1), None);
        plan.get_mut(music).unwrap().flow.repeat = Repeat::Times(2);
        assert_eq!(curl_at(&plan, 1), Some(Curl::Into));
        assert_eq!(curl_at(&plan, 2), Some(Curl::Past));
        assert_eq!(curl_at(&plan, 3), Some(Curl::Out));
        assert_eq!(curl_at(&plan, 5), None, "Sermon doesn't repeat");
        // One step has nothing to curl round.
        plan.get_mut(sermon).unwrap().flow.repeat = Repeat::Forever;
        assert_eq!(curl_at(&plan, 5), None);
        // The plan repeats: the steps in nothing nearer are its own.
        plan.get_mut(sermon).unwrap().flow.repeat = Repeat::Once;
        plan.get_mut(music).unwrap().flow.repeat = Repeat::Once;
        plan.flow.repeat = Repeat::Forever;
        assert_eq!(curl_at(&plan, 1), Some(Curl::Into));
        assert_eq!(curl_at(&plan, 5), Some(Curl::Out));
    }
}
