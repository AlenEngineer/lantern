//! The run view's left pane: the plan. Its steps, grouped under section
//! headers that say how each part of the service is keeping to its time,
//! and the plan's files (open, save, new).
//!
//! Clicking a step selects it and lines it up in the preview; Enter puts it
//! on stage. Editing the plan is the prepare view's job.

use std::time::{Duration, SystemTime};

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Fill};
use lantern_engine::{Action, Basis, Pace, Phase, Plan, Query, SectionTiming, Step, Timing};

use crate::dashboard::{NAME_BOX, chip, op, pill, scrolling, single_line, wide};
use crate::keys::Op;
use crate::length;
use crate::look::{self, BODY, BOLD, HEADING, MEDIUM, META, PILL_SMALL, Tone};
use crate::palette::Palette;
use crate::plans::{self, Entry, Notice, Switch, file_name};
use crate::{App, Message};

const PLAN_WIDTH: f32 = 280.0;
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
        let mut pane = column![self.plan_header(&plan)].spacing(8);
        if let Some(notice) = self.plan_notice(&plan) {
            pane = pane.push(notice);
        }
        pane = pane.push(match &self.plans.mode {
            plans::Mode::Steps | plans::Mode::Renaming(_) => self.steps(&plan, timing),
            plans::Mode::Browsing(entries) => self.saved_plans(entries),
            plans::Mode::Confirm(switch) => confirm(p, &plan, switch),
        });
        let small = |label, message| wide(pill(p, label, Tone::Plain, PILL_SMALL, Some(message)));
        let buttons = match &self.plans.mode {
            plans::Mode::Browsing(_) => row![small("Cancel", Message::ClosePlanMode)],
            _ => row![
                small("Open", op(Op::Open)),
                small("Save", op(Op::Save)),
                small("New", op(Op::New)),
            ],
        };
        pane.push(buttons.spacing(4))
            .padding([16, 12])
            .width(PLAN_WIDTH)
            .height(Fill)
            .into()
    }

    /// The plan's name (click to rename), and where the service is: "3 of 7".
    fn plan_header(&self, plan: &Plan) -> Element<'_, Message> {
        let p = self.palette;
        let name: Element<'_, Message> = match &self.plans.mode {
            plans::Mode::Renaming(draft) => text_input("Plan name", draft)
                .id(NAME_BOX)
                .on_input(Message::PlanName)
                .on_submit(Message::SubmitPlanName)
                .size(BODY)
                .padding([6, 10])
                .style(look::input(p))
                .into(),
            _ => button(
                single_line(plan.title.clone())
                    .size(HEADING)
                    .font(BOLD)
                    .color(p.primary),
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
        let mut place = match at {
            Some(at) => format!("{} of {}", at + 1, items.len()),
            None => count(items.len(), "step"),
        };
        if self.plans.is_dirty(plan) {
            place.push_str(" · unsaved");
        }
        row![name, text(place).size(META).color(p.on_surface_variant)]
            .spacing(6)
            .align_y(Alignment::Center)
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
        Some(
            container(text(message.as_str()).size(META).color(color))
                .padding([0, 8])
                .into(),
        )
    }

    fn steps(&self, plan: &Plan, timing: &Timing) -> Element<'_, Message> {
        let p = self.palette;
        let current = self.engine.0.current_step().map(|step| step.id);
        let next = self.engine.0.up_next().map(|step| step.id);
        let at = current.and_then(|id| plan.position(id));
        let mut rows = column![].spacing(2);
        let mut number = 0;
        let mut in_section = false;
        for (index, step) in plan.steps().iter().enumerate() {
            if step.is_section() {
                in_section = true;
                let section = timing.sections.iter().find(|s| s.id == step.id);
                rows = rows.push(self.section_row(step, section));
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
            let row = self.step_row(number, step, place);
            rows = rows.push(if in_section {
                container(row).padding(iced::Padding::default().left(INDENT))
            } else {
                container(row)
            });
        }
        if plan.is_empty() {
            rows = rows.push(
                container(
                    text("The plan is empty. Preview something and press Ctrl+A to add it.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        scrolling(rows).into()
    }

    /// A section's header: its name, its time against its budget, and how
    /// it's doing. Clicking it goes to its first step.
    fn section_row(&self, step: &Step, timing: Option<&SectionTiming>) -> Element<'_, Message> {
        let p = self.palette;
        let (time, pace) = timing.map_or((String::new(), None), |t| (section_time(t), t.pace()));
        let mut line = row![
            container(
                single_line(step.label.clone())
                    .size(BODY)
                    .font(BOLD)
                    .color(p.primary)
            )
            .width(Fill)
            .clip(true),
            text(time).size(META).color(p.on_surface_variant),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(pace) = pace {
            line = line.push(chip(p, pace_word(pace), pace_tone(pace)));
        }
        button(line)
            .width(Fill)
            .padding([10, 10])
            .on_press(Message::Select(step.id))
            .style(look::row(p, Tone::Plain))
            .into()
    }

    fn step_row(&self, number: usize, step: &Step, place: Place) -> Element<'_, Message> {
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
        let (marker, note) = match place {
            Place::Current if shown => ("●".to_string(), Some("on stage".to_string())),
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
        let detail = note.or_else(|| match &step.action {
            Action::Show { resource } => self.download_state(resource),
            Action::Live { hint, shown } => Some(slot_detail(hint.as_ref(), shown.len())),
            Action::Blank => Some("black screen".into()),
            Action::Note => Some("note".into()),
            Action::Section { .. } => None,
        });
        if let Some(detail) = detail {
            label = label.push(single_line(detail).size(META).color(p.on_surface_variant));
        }
        let line = row![
            text(marker).size(META).color(label_color).width(16),
            container(label).width(Fill).clip(true),
            text(self.step_length(step))
                .size(META)
                .color(p.on_surface_variant),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        button(line)
            .width(Fill)
            .padding([8, 10])
            .on_press(Message::Select(step.id))
            .style(look::row(p, tone))
            .into()
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
        let folder = self.plans.store().dir().display().to_string();
        let mut list = column![
            container(
                column![
                    text("Open a saved plan")
                        .size(BODY)
                        .font(BOLD)
                        .color(p.primary),
                    single_line(folder).size(META).color(p.on_surface_variant),
                ]
                .spacing(2)
            )
            .padding([4, 8])
        ]
        .spacing(2);
        if entries.is_empty() {
            list = list.push(
                container(
                    text("No saved plans yet. Save makes one.")
                        .size(BODY)
                        .color(p.on_surface_variant),
                )
                .padding([4, 8]),
            );
        }
        let now = SystemTime::now();
        for entry in entries {
            let when = now.duration_since(entry.modified).unwrap_or_default();
            list = list.push(saved_plan(p, entry, ago(when)));
        }
        scrolling(list).into()
    }
}

/// A section's time: `6:40 / 15:00` while it runs, `took 14:20` once done,
/// `~12 min / 15:00` before it starts.
fn section_time(section: &SectionTiming) -> String {
    let of = |time: String| match section.budget {
        Some(budget) => format!("{time} / {}", length::clock(budget)),
        None => time,
    };
    match section.phase {
        Phase::Coming { planned } if planned.is_zero() => of("empty".into()),
        Phase::Coming { planned } => of(length::rough(planned)),
        Phase::Running { elapsed, .. } => of(length::clock(elapsed)),
        Phase::Done { took: Some(took) } => format!("took {}", length::clock(took)),
        Phase::Done { took: None } => "done".into(),
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

fn saved_plan(p: Palette, entry: &Entry, when: String) -> Element<'_, Message> {
    let (title, detail, color) = match &entry.summary {
        Ok((title, steps)) => (
            title.clone(),
            format!("{} · {when}", count(*steps, "step")),
            p.on_surface,
        ),
        Err(why) => (
            file_name(&entry.path),
            format!("can't read: {why}"),
            p.error,
        ),
    };
    button(
        column![
            single_line(title).size(BODY).font(MEDIUM).color(color),
            single_line(detail).size(META).color(p.on_surface_variant),
        ]
        .spacing(2),
    )
    .width(Fill)
    .padding([8, 10])
    .on_press_maybe(
        entry
            .summary
            .is_ok()
            .then(|| Message::OpenPlan(entry.path.clone())),
    )
    .style(look::row(p, Tone::Plain))
    .into()
}

fn confirm<'a>(p: Palette, plan: &Plan, switch: &Switch) -> Element<'a, Message> {
    let then = match switch {
        Switch::New => "start a new one",
        Switch::Open(_) => "open another",
    };
    column![
        text(format!(
            "“{}” was never saved. Discard it and {then}?",
            plan.title
        ))
        .size(BODY),
        text("Ctrl+S saves it instead.")
            .size(META)
            .color(p.on_surface_variant),
        row![
            wide(pill(
                p,
                "Discard",
                Tone::Danger,
                PILL_SMALL,
                Some(Message::DiscardPlan)
            )),
            wide(pill(
                p,
                "Keep it",
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
    fn sections_say_their_time_against_their_budget() {
        let coming = section(Some(15 * MIN), Phase::Coming { planned: 12 * MIN });
        assert_eq!(section_time(&coming), "~12 min / 15:00");
        let running = Phase::Running {
            elapsed: 6 * MIN + Duration::from_secs(40),
            expected: 12 * MIN,
        };
        assert_eq!(
            section_time(&section(Some(15 * MIN), running)),
            "6:40 / 15:00"
        );
        let done = Phase::Done {
            took: Some(14 * MIN),
        };
        assert_eq!(section_time(&section(None, done)), "took 14:00");
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
}
