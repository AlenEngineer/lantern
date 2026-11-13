//! The left pane: the plan. Its steps, grouped under section headers that
//! say how each part is keeping to its time, edited in place (see
//! [`crate::plan_edit`]), and behind the Plans button the saved plans.
//!
//! Clicking a step selects it, opens its edit strip and lines it up in the
//! preview; Enter puts it on stage, and Ctrl+A adds after it.

use std::time::{Duration, SystemTime};

use iced::widget::{
    Space, button, column, container, mouse_area, row, stack, text, text_input, tooltip,
};
use iced::{Alignment, Element, Fill, Length, mouse};
use lantern_engine::{
    Action, Basis, Media, Pace, Phase, Plan, Query, Resource, SectionTiming, Step, StepId, Timing,
};

use crate::dashboard::{
    NAME_BOX, STEP_THUMB, level_tone, op, pill, scrolling, single_line, unit, wide,
};
use crate::drag::{Drag, Half};
use crate::fit;
use crate::keys::Op;
use crate::length;
use crate::look::{self, BODY, BOLD, HEADING, Icon, MEDIUM, META, PILL_SMALL, Tone};
use crate::palette::Palette;
use crate::plan_edit::{NewStep, insertion_mark};
use crate::plans::{self, Entry, Notice, Switch, file_name};
use crate::ready::Issue;
use crate::{App, Clicked, Message};

/// How far a section's steps sit in from its header.
const INDENT: f32 = 12.0;

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
        let mut pane = column![self.plan_header(&plan)].spacing(8);
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
        let small = |label, message| pill(p, label, Tone::Plain, PILL_SMALL, Some(message));
        let buttons = match &self.plans.mode {
            plans::Mode::Browsing(_) => row![wide(small("New plan · Ctrl+N", op(Op::New)))],
            _ => {
                let add = |label, new| small(label, Message::AddStep(new));
                row![
                    add("Section", NewStep::Section),
                    add("Live slot", NewStep::LiveSlot),
                    add("Black", NewStep::BlackScreen),
                    add("Note", NewStep::Note),
                ]
                .align_y(Alignment::Center)
            }
        };
        pane.push(buttons.spacing(4))
            .padding([16, 12])
            .width(self.plan_width())
            .height(Fill)
            .into()
    }

    /// The Plans button, the plan's name (click to rename), and where the
    /// service is: "3 of 7".
    fn plan_header(&self, plan: &Plan) -> Element<'_, Message> {
        let p = self.palette;
        let browsing = matches!(self.plans.mode, plans::Mode::Browsing(_));
        let plans_button = pill(
            p,
            if browsing { "‹ Back" } else { "Plans" },
            if browsing {
                Tone::Selected
            } else {
                Tone::Plain
            },
            PILL_SMALL,
            Some(op(Op::Open)),
        );
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
        let items: Vec<&Step> = plan.steps().iter().filter(|s| !s.is_section()).collect();
        let at = self
            .engine
            .0
            .current_step()
            .and_then(|current| items.iter().position(|s| s.id == current.id));
        let place = match at {
            Some(at) => format!("{} of {}", at + 1, items.len()),
            None => count(items.len(), "step"),
        };
        let mut header = row![
            plans_button,
            name,
            text(place).size(META).color(p.on_surface_variant),
            self.look_button(plan),
        ];
        if self.plans.is_dirty(plan) {
            header = header.push(pill(
                p,
                "Save",
                Tone::Selected,
                PILL_SMALL,
                Some(op(Op::Save)),
            ));
        }
        header.spacing(6).align_y(Alignment::Center).into()
    }

    /// A swatch of the stage's look for this plan (its background, ringed
    /// in its text colour) that opens the settings on this plan's look.
    fn look_button(&self, plan: &Plan) -> Element<'_, Message> {
        let p = self.palette;
        let (background, ink) = (fit::color(self.look.background), fit::color(self.look.text));
        let swatch =
            container(Space::new().width(14).height(14)).style(move |_| container::Style {
                background: Some(background.into()),
                border: iced::Border {
                    color: ink,
                    width: 2.0,
                    radius: 7.0.into(),
                },
                ..container::Style::default()
            });
        // The swatch alone was a circle nobody could name: it says "Look".
        let label = row![swatch, text("Look").size(META).color(p.on_surface_variant)]
            .spacing(6)
            .align_y(Alignment::Center);
        let about = if plan.look.is_some() {
            "This plan's own stage look"
        } else {
            "Stage look (the one for all plans)"
        };
        tooltip(
            button(label)
                .padding([4, 8])
                .on_press(Message::PlanLook)
                .style(look::row(p, Tone::Plain)),
            container(text(about).size(META))
                .padding([4, 8])
                .style(look::tip(p)),
            tooltip::Position::Bottom,
        )
        .into()
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
        Some(single_line(message.as_str()).size(META).color(color).into())
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
        let landing = moving.and_then(|drag| drag.landing);
        let gap = |indent: f32| {
            container(drop_gap(
                p,
                moving.map(|d| d.label(plan)).unwrap_or_default(),
            ))
            .padding(iced::Padding::default().left(indent))
        };
        let mut rows = column![].spacing(2);
        let mut number = 0;
        let mut in_section = false;
        let mut rest = 0;
        for (index, step) in plan.steps().iter().enumerate() {
            if carried.contains(&step.id) {
                continue;
            }
            if landing == Some(rest) {
                rows = rows.push(gap(gap_indent(moving, in_section)));
            }
            rest += 1;
            let selected = self.selected == Some(step.id) && moving.is_none();
            if step.is_section() {
                in_section = true;
                let section = timing.sections.iter().find(|s| s.id == step.id);
                rows = rows.push(self.draggable(step.id, self.section_row(plan, step, section)));
                if selected {
                    rows = rows
                        .push(self.edit_strip(step))
                        .push(insertion_mark(p, INDENT));
                }
                continue;
            }
            number += 1;
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
            let mut row =
                column![self.draggable(step.id, self.step_row(number, step, place, issue))];
            if selected {
                row = row.push(self.edit_strip(step)).push(insertion_mark(p, 0.0));
            } else if self.selected.is_none() && Some(step.id) == current && moving.is_none() {
                // With nothing selected, a new step goes after the current one.
                row = row.push(insertion_mark(p, 0.0));
            }
            let indent = if in_section { INDENT } else { 0.0 };
            rows = rows.push(container(row).padding(iced::Padding::default().left(indent)));
        }
        if landing == Some(rest) {
            rows = rows.push(gap(gap_indent(moving, in_section)));
        }
        if self.selected.is_none() && current.is_none() && !plan.is_empty() && moving.is_none() {
            rows = rows.push(insertion_mark(p, 0.0));
        }
        if plan.is_empty() {
            rows = rows.push(
                container(
                    text("The plan is empty. Search, then Ctrl+A adds the result here.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        // Leaving the list means letting go would move nothing.
        mouse_area(scrolling(rows))
            .on_exit(Message::LeftPlan)
            .into()
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
    ) -> Element<'_, Message> {
        let p = self.palette;
        let id = step.id;
        let budget = match step.action {
            Action::Section { budget } => budget,
            _ => None,
        };
        let phase = timing.map_or(
            Phase::Coming {
                planned: self.planned(plan, id),
            },
            |t| t.phase,
        );
        let time = section_time(budget, &phase);
        let mut line = row![
            container(
                single_line(step.label.clone())
                    .size(BODY)
                    .font(BOLD)
                    .color(p.primary)
            )
            .width(Fill)
            .clip(true),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let color = match time.pace {
            Some(pace) if pace != Pace::OnTrack => pace_tone(pace).text(p),
            _ => p.on_surface_variant,
        };
        if !time.words.is_empty() {
            line = line.push(
                row![
                    look::icon(Icon::Clock, 13.0, color),
                    text(time.words.clone()).size(META).color(color)
                ]
                .spacing(4)
                .align_y(Alignment::Center),
            );
        }
        if time.offer {
            line = line.push(pill(
                p,
                "Set a time",
                Tone::Plain,
                PILL_SMALL,
                Some(Message::SetTime(id)),
            ));
        }
        let content: Element<'_, Message> = match time.share {
            Some(share) => {
                let fill = match (time.pace, &phase) {
                    (Some(pace), _) if pace != Pace::OnTrack => pace_tone(pace).text(p),
                    (_, Phase::Running { .. }) => p.tertiary,
                    _ => p.primary,
                };
                column![line, bar(p, share, fill, 0.0)].spacing(6).into()
            }
            None => line.into(),
        };
        let tone = if self.selected == Some(id) {
            Tone::Selected
        } else {
            Tone::Plain
        };
        button(content)
            .width(Fill)
            .padding([10, 10])
            .on_press(Message::Click(Clicked::Step(id)))
            .style(look::row(p, tone))
            .into()
    }

    fn step_row(
        &self,
        number: usize,
        step: &Step,
        place: Place,
        issue: Option<&Issue>,
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
            _ if self.selected == Some(step.id) => Tone::Selected,
            _ => Tone::Plain,
        };
        // On stage, the row says how far into it the stage is.
        let progress = match (place, &self.stage.program) {
            (Place::Current, Some(scene)) if shown => progress(&scene.media, &scene.resource),
            _ => None,
        };
        let (marker, note) = match place {
            Place::Current if shown => (
                "●".to_string(),
                Some(match &progress {
                    Some((how_far, _)) => format!("on stage · {how_far}"),
                    None => "on stage".to_string(),
                }),
            ),
            // A live slot under way says what it searches and showed.
            Place::Current => match &step.action {
                Action::Live { hint, shown } => (
                    "●".to_string(),
                    Some(slot_detail(hint.as_ref(), shown.len())),
                ),
                _ => ("●".to_string(), Some("current step".to_string())),
            },
            Place::Next => ("○".to_string(), Some("next, PgDn".to_string())),
            Place::Done | Place::Later => (number.to_string(), None),
        };
        let label_color = match (place, tone) {
            (Place::Done, _) => p.outline,
            (_, Tone::Plain) => p.on_surface,
            (_, tone) => tone.text(p),
        };
        let mut label = column![
            single_line(step.label.clone())
                .size(BODY)
                .font(if tone == Tone::Plain { MEDIUM } else { BOLD })
                .color(label_color)
        ];
        // What's wrong with a step comes before anything else about it.
        let (detail, detail_color) = match (note, issue) {
            (Some(note), _) => (Some(note), p.on_surface_variant),
            (None, Some(issue)) => (
                Some(issue.what.clone()),
                level_tone(Some(issue.level)).text(p),
            ),
            (None, None) => (
                match &step.action {
                    Action::Show { resource } => self.download_state(resource),
                    Action::Live { hint, shown } => Some(slot_detail(hint.as_ref(), shown.len())),
                    Action::Blank => Some("black screen".into()),
                    Action::Note => Some("note".into()),
                    Action::Section { .. } => None,
                },
                p.on_surface_variant,
            ),
        };
        // A step that goes on by itself says so, and counts down once up.
        let detail = match (step.advance, place) {
            (false, _) => detail,
            (true, Place::Current) => match self.goes_on_in() {
                Some(left) => Some(join(detail, format!("goes on in {}", seconds_left(left)))),
                None => Some(join(detail, "goes on when it ends".into())),
            },
            (true, _) => Some(join(detail, "goes on".into())),
        };
        // "Note / note" says nothing twice: the kind shows once renamed.
        let detail = detail.filter(|detail| !detail.eq_ignore_ascii_case(step.label.trim()));
        if let Some(detail) = detail {
            label = label.push(single_line(detail).size(META).color(detail_color));
        }
        // Every row has a picture, so the titles line up: a step that shows
        // nothing of its own has its kind's icon, a black screen is black.
        let kind = |icon: Option<Icon>| -> Element<'_, Message> {
            let height = STEP_THUMB * 9.0 / 16.0;
            let inside: Element<'_, Message> = match icon {
                Some(icon) => look::icon(icon, height * 0.4, p.on_surface_variant).into(),
                None => Space::new().into(),
            };
            let black = icon.is_none();
            container(inside)
                .center(Fill)
                .width(STEP_THUMB)
                .height(height)
                .style(move |theme| {
                    let style = look::thumb(p)(theme);
                    if black {
                        style.background(iced::Color::BLACK)
                    } else {
                        style
                    }
                })
                .into()
        };
        let picture: Element<'_, Message> = match &step.action {
            Action::Show { resource } => self.thumbnail(resource, STEP_THUMB),
            Action::Live { .. } => kind(Some(Icon::Search)),
            Action::Blank => kind(None),
            Action::Note | Action::Section { .. } => kind(Some(Icon::Document)),
        };
        let line = row![
            text(marker).size(META).color(label_color).width(16),
            picture,
            container(label).width(Fill).clip(true),
            text(self.step_length(step))
                .size(META)
                .color(p.on_surface_variant),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let row = button(line)
            .width(Fill)
            .padding([8, 10])
            .on_press(Message::Click(Clicked::Step(step.id)))
            .style(look::row(p, tone));
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

    /// How long a step takes: `4:00` when known, `~4 min` when guessed,
    /// nothing for a step that shows nothing.
    fn step_length(&self, step: &Step) -> String {
        let estimate = lantern_engine::timing::estimate(step, self.loaded_length(step));
        match estimate.basis {
            Basis::Guess if estimate.length.is_zero() => String::new(),
            Basis::Guess => length::rough(estimate.length),
            Basis::Set | Basis::Known => length::clock(estimate.length),
        }
    }

    fn saved_plans<'a>(&self, entries: &'a [Entry]) -> Element<'a, Message> {
        let p = self.palette;
        let heading = |title: &'static str, about: String| {
            container(
                column![
                    text(title).size(BODY).font(BOLD).color(p.primary),
                    single_line(about).size(META).color(p.on_surface_variant),
                ]
                .spacing(2),
            )
            .padding([4, 8])
        };
        let is_template = |entry: &Entry| entry.summary.as_ref().is_ok_and(|s| s.template);
        let now = SystemTime::now();
        let when = |entry: &Entry| ago(now.duration_since(entry.modified).unwrap_or_default());
        let mut list = column![].spacing(2);
        let templates: Vec<&Entry> = entries.iter().filter(|e| is_template(e)).collect();
        if !templates.is_empty() {
            list = list.push(heading(
                "Templates",
                "Click one to start a new plan from it".into(),
            ));
            for entry in templates {
                list = list.push(saved_plan(p, entry, when(entry)));
            }
            list = list.push(Space::new().height(10));
        }
        // Where the files are is in the README, for whoever goes looking.
        list = list.push(heading("Your plans", "Click one to open it".into()));
        let plans: Vec<&Entry> = entries.iter().filter(|e| !is_template(e)).collect();
        if plans.is_empty() {
            list = list.push(
                container(
                    text("No saved plans yet. Save makes one; ☆ makes a saved plan a template.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        for entry in plans {
            list = list.push(saved_plan(p, entry, when(entry)));
        }
        scrolling(list).into()
    }
}

/// What a section's header says about its time.
#[derive(Debug, Clone, PartialEq)]
struct SectionTime {
    /// "12 of 15 min planned", "6:40 of 15 min", "took 14:20".
    words: String,
    /// How much of the time is used, for the bar; `None` without a time.
    share: Option<f32>,
    pace: Option<Pace>,
    /// No time yet and not started: offer to set one.
    offer: bool,
}

fn section_time(budget: Option<Duration>, phase: &Phase) -> SectionTime {
    let share = |used: Duration| budget.map(|b| used.as_secs_f32() / b.as_secs_f32().max(1.0));
    let of = |used: String| match budget {
        Some(budget) => format!("{used} of {}", minutes(budget)),
        None => used,
    };
    match *phase {
        Phase::Coming { planned } => {
            let pace = budget
                .filter(|_| !planned.is_zero())
                .map(|b| Pace::of(planned, b));
            let mut words = match (budget, planned.is_zero()) {
                (None, true) => String::new(),
                (None, false) => format!("{} planned", length::rough(planned)),
                (Some(_), true) => of("0".into()) + " planned",
                (Some(_), false) => {
                    of(length::rough(planned).trim_start_matches('~').to_string()) + " planned"
                }
            };
            if let (Some(Pace::Over), Some(budget)) = (pace, budget) {
                words.push_str(&format!(" · {} over", minutes(planned - budget)));
            }
            SectionTime {
                words,
                share: share(planned),
                pace,
                offer: budget.is_none(),
            }
        }
        Phase::Running { elapsed, expected } => {
            let pace = budget.map(|b| Pace::of(expected, b));
            let mut words = of(length::clock(elapsed));
            if let Some(pace) = pace {
                words.push_str(" · ");
                words.push_str(pace_word(pace));
            }
            SectionTime {
                words,
                share: share(elapsed),
                pace,
                offer: false,
            }
        }
        Phase::Done { took } => SectionTime {
            words: match took {
                Some(took) => of(format!("took {}", length::clock(took))),
                None => "done".into(),
            },
            share: None,
            pace: budget.zip(took).map(|(b, took)| Pace::of(took, b)),
            offer: false,
        },
    }
}

/// A time as a person says it: "15 min", "1 h 30 min", "4:30".
fn minutes(time: Duration) -> String {
    let secs = time.as_secs();
    match (secs / 3600, secs % 3600 / 60, secs % 60) {
        (0, m, 0) => format!("{m} min"),
        (h, 0, 0) => format!("{h} h"),
        (h, m, 0) => format!("{h} h {m} min"),
        _ => length::clock(time),
    }
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
fn bar<'a>(p: Palette, done: f32, color: iced::Color, indent: f32) -> Element<'a, Message> {
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

fn pace_word(pace: Pace) -> &'static str {
    match pace {
        Pace::OnTrack => "on track",
        Pace::Tight => "tight",
        Pace::Over => "over",
    }
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
fn saved_plan(p: Palette, entry: &Entry, when: String) -> Element<'_, Message> {
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
    .style(look::row(p, Tone::Plain));
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
    );
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
            "What's on stage stays there. The plan is in Plans to come back to.",
            "Leave it",
            Message::LeavePlan,
            "Stay",
        )
    } else {
        (
            format!("“{}” was never saved. Discard it and {then}?", plan.title),
            "Ctrl+S saves it instead.",
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
fn gap_indent(moving: Option<&Drag>, in_section: bool) -> f32 {
    let step = moving.is_some_and(|drag| drag.carried.len() == 1 && !drag.is_section());
    if step && in_section { INDENT } else { 0.0 }
}

/// Where a drag would land: a highlighted slot naming what it carries.
fn drop_gap<'a>(p: Palette, label: String) -> Element<'a, Message> {
    container(text(label).size(BODY).font(MEDIUM).color(p.primary))
        .width(Fill)
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

/// `detail · more`, or just `more`.
fn join(detail: Option<String>, more: String) -> String {
    match detail {
        Some(detail) => format!("{detail} · {more}"),
        None => more,
    }
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
    fn sections_say_their_time_in_words_or_offer_to_set_one() {
        let coming = |planned| Phase::Coming { planned };
        let budget = Some(15 * MIN);

        let empty = section_time(None, &coming(Duration::ZERO));
        assert_eq!((empty.words.as_str(), empty.offer), ("", true));
        let unset = section_time(None, &coming(12 * MIN));
        assert_eq!(
            (unset.words.as_str(), unset.offer),
            ("~12 min planned", true)
        );

        let none_yet = section_time(budget, &coming(Duration::ZERO));
        assert_eq!(none_yet.words, "0 of 15 min planned");
        assert!(!none_yet.offer);
        let planned = section_time(budget, &coming(12 * MIN));
        assert_eq!(planned.words, "12 min of 15 min planned");
        assert_eq!(planned.share, Some(0.8));
        let over = section_time(budget, &coming(19 * MIN));
        assert_eq!(over.words, "19 min of 15 min planned · 4 min over");
        assert_eq!(over.pace, Some(Pace::Over));

        let running = Phase::Running {
            elapsed: 6 * MIN + Duration::from_secs(40),
            expected: 12 * MIN,
        };
        assert_eq!(
            section_time(budget, &running).words,
            "6:40 of 15 min · on track"
        );
        let done = Phase::Done {
            took: Some(14 * MIN),
        };
        assert_eq!(section_time(None, &done).words, "took 14:00");
        assert_eq!(minutes(90 * MIN), "1 h 30 min");
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
}
