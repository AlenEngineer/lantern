//! The prepare view (Ctrl+R): building the plan ahead of the service.
//!
//! The plan as a document on the left, every name, length and budget a
//! field, and the launcher on the right, where Enter adds the result to the
//! plan instead of showing it. New steps go right after the selected one.
//! Nothing here reaches the projector.

use std::time::Duration;

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Fill};
use lantern_engine::{Action, Basis, Pace, Plan, Step, StepId};

use crate::dashboard::{chip, op, pill, scrolling, single_line};
use crate::keys::Op;
use crate::length;
use crate::look::{self, BODY, BOLD, Icon, MEDIUM, META, PILL_SMALL, Tone};
use crate::palette::Palette;
use crate::plan_pane::count;
use crate::{App, Message};

const SEARCH_WIDTH: f32 = 560.0;
const LENGTH_FIELD: f32 = 56.0;
const BUDGET_FIELD: f32 = 48.0;
const TITLE_SIZE: f32 = 22.0;

/// Which of the window's two jobs it's doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum View {
    /// Running the service: the stage, the plan, and searching on the fly.
    #[default]
    Run,
    /// Building the plan ahead of it.
    Prepare,
}

impl View {
    pub fn toggled(self) -> View {
        match self {
            View::Run => View::Prepare,
            View::Prepare => View::Run,
        }
    }
}

/// A step the "Add here" buttons make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewStep {
    Section,
    LiveSlot,
    BlackScreen,
    Note,
}

impl NewStep {
    /// Its label and action, ready to rename.
    pub fn step(self) -> (&'static str, Action) {
        match self {
            NewStep::Section => (
                "New section",
                Action::Section {
                    budget: Some(Duration::from_secs(15 * 60)),
                },
            ),
            NewStep::LiveSlot => (
                "Live search",
                Action::Live {
                    hint: None,
                    shown: Vec::new(),
                },
            ),
            NewStep::BlackScreen => ("Black screen", Action::Blank),
            NewStep::Note => ("Note", Action::Note),
        }
    }
}

impl App {
    /// The switch between the two views, at the left of each view's header.
    pub(crate) fn view_switch(&self) -> Element<'_, Message> {
        let p = self.palette;
        // The current view is a label, the other one a button to it.
        let option = |label: &'static str, view: View| -> Element<'_, Message> {
            if self.view == view {
                container(text(label).size(BODY).font(BOLD))
                    .height(PILL_SMALL)
                    .padding([0, 16])
                    .center_y(PILL_SMALL)
                    .style(look::chip(p, Tone::Selected))
                    .into()
            } else {
                pill(p, label, Tone::Plain, PILL_SMALL, Some(op(Op::ToggleView))).into()
            }
        };
        container(row![option("Prepare", View::Prepare), option("Run", View::Run)].spacing(4))
            .padding(3)
            .style(look::chip(p, Tone::Plain))
            .into()
    }

    pub(crate) fn prepare_view(&self) -> Element<'_, Message> {
        let p = self.palette;
        let small = |label, message| pill(p, label, Tone::Plain, PILL_SMALL, Some(message));
        let header = row![
            look::icon(Icon::Lantern, 20.0, p.primary),
            text("Lantern")
                .size(look::HEADING)
                .font(BOLD)
                .color(p.primary),
            self.view_switch(),
            text("Ctrl+R run").size(META).color(p.on_surface_variant),
            Space::new().width(Fill),
            small("Open", op(Op::Open)),
            small("Save", op(Op::Save)),
            small("New", op(Op::New)),
            button(look::icon(Icon::Settings, 18.0, p.on_surface_variant))
                .padding(8)
                .on_press(op(Op::Settings))
                .style(look::pill(p, Tone::Plain)),
        ]
        .spacing(10)
        .align_y(Alignment::Center)
        .padding([10, 16]);
        let seam = |vertical: bool| {
            let line = container(Space::new()).style(look::hairline(p));
            if vertical {
                line.width(1).height(Fill)
            } else {
                line.height(1).width(Fill)
            }
        };
        let side: Element<'_, Message> = match &self.settings {
            Some(draft) => self.settings_view(draft),
            None => self.prepare_search(),
        };
        column![
            header,
            seam(false),
            row![
                self.document(),
                seam(true),
                container(side)
                    .width(SEARCH_WIDTH)
                    .height(Fill)
                    .padding([20, 24]),
            ]
            .height(Fill),
        ]
        .into()
    }

    /// The plan as a document: its name, every step, and the buttons that
    /// add steps where the selection is.
    fn document(&self) -> Element<'_, Message> {
        let p = self.palette;
        let plan = self.engine.0.plan();
        let total: Duration = plan
            .steps()
            .iter()
            .filter(|s| !s.is_section())
            .map(|s| self.estimate(s).length)
            .sum();
        let steps = plan.steps().iter().filter(|s| !s.is_section()).count();
        let mut summary = format!("{} · {}", count(steps, "step"), length::rough(total));
        if self.plans.is_dirty(&plan) {
            summary.push_str(" · unsaved");
        }
        let title = text_input("Plan name", &plan.title)
            .on_input(Message::PlanTitle)
            .size(TITLE_SIZE)
            .font(BOLD)
            .padding([4, 0])
            .style(bare(p, p.primary));
        let mut rows = column![].spacing(2);
        for step in plan.steps() {
            rows = rows.push(if step.is_section() {
                self.section_line(&plan, step)
            } else {
                self.step_line(step)
            });
            if self.selected == Some(step.id) {
                rows = rows.push(insertion_mark(p));
            }
        }
        if self.selected.is_none() || plan.is_empty() {
            rows = rows.push(insertion_mark(p));
        }
        let add = |label, new| {
            pill(
                p,
                label,
                Tone::Plain,
                PILL_SMALL,
                Some(Message::AddStep(new)),
            )
        };
        let adds = row![
            text("Add here").size(META).color(p.on_surface_variant),
            add("Section", NewStep::Section),
            add("Live slot", NewStep::LiveSlot),
            add("Black screen", NewStep::BlackScreen),
            add("Note", NewStep::Note),
            Space::new().width(Fill),
            text("Alt+↑↓ move · Ctrl+R run")
                .size(META)
                .color(p.on_surface_variant),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        column![
            row![
                title.width(Fill),
                text(summary).size(META).color(p.on_surface_variant)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
            scrolling(rows),
            adds,
        ]
        .spacing(10)
        .padding([20, 28])
        .width(Fill)
        .height(Fill)
        .into()
    }

    /// A section's header: its name and budget as fields, and what its steps
    /// add up to against the budget.
    fn section_line(&self, plan: &Plan, section: &Step) -> Element<'_, Message> {
        let p = self.palette;
        let id = section.id;
        let budget = match section.action {
            Action::Section { budget } => budget,
            _ => None,
        };
        let planned = self.planned(plan, id);
        let name = text_input("Section", &section.label)
            .on_input(move |label| Message::Rename(id, label))
            .size(look::HEADING)
            .font(BOLD)
            .padding([4, 0])
            .style(bare(p, p.primary));
        let mut line = row![
            name.width(Fill),
            text(if planned.is_zero() {
                "nothing yet, of".to_string()
            } else {
                format!("{} of", length::rough(planned))
            })
            .size(META)
            .color(p.on_surface_variant),
            self.length_field(id, budget, "15".into(), BUDGET_FIELD),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if let Some(budget) = budget {
            let pace = Pace::of(planned, budget);
            if pace != Pace::OnTrack {
                let over = planned.saturating_sub(budget);
                let (label, tone) = match pace {
                    Pace::Over => (
                        format!("{} over", length::rough(over).trim_start_matches('~')),
                        Tone::Danger,
                    ),
                    _ => ("tight".to_string(), Tone::Live),
                };
                line = line.push(chip(p, &label, tone));
            }
        }
        self.selectable(id, line, true)
    }

    /// A step: its kind, name, what it is, and its length.
    fn step_line(&self, step: &Step) -> Element<'_, Message> {
        let p = self.palette;
        let id = step.id;
        let selected = self.selected == Some(id);
        let (icon, about) = match &step.action {
            Action::Show { resource } => {
                let mut about: Vec<String> = resource.subtitle.iter().cloned().collect();
                about.extend(self.download_state(resource));
                (Icon::of(&resource.kind), about.join(" · "))
            }
            Action::Live { .. } => (Icon::Search, "live slot".into()),
            Action::Blank => (Icon::Document, "black screen".into()),
            Action::Note => (Icon::Document, "note".into()),
            Action::Section { .. } => (Icon::Document, String::new()),
        };
        let tone = if selected {
            Tone::Selected
        } else {
            Tone::Plain
        };
        let name = text_input("Step", &step.label)
            .on_input(move |label| Message::Rename(id, label))
            .size(BODY)
            .font(if selected { BOLD } else { MEDIUM })
            .padding([4, 0])
            .style(bare(p, if selected { p.primary } else { p.on_surface }));
        let estimate = self.estimate(step);
        let shown = match estimate.basis {
            Basis::Guess if estimate.length.is_zero() => String::new(),
            Basis::Guess => length::rough(estimate.length),
            Basis::Set | Basis::Known => length::clock(estimate.length),
        };
        let line = row![
            disc(p, icon, tone),
            name.width(Fill),
            container(single_line(about).size(META).color(p.on_surface_variant))
                .max_width(220)
                .clip(true),
            self.length_field(id, step.estimate, shown, LENGTH_FIELD),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        self.selectable(id, line, false)
    }

    /// A row that selects its step when clicked, tinted while selected,
    /// with buttons to move it or take it out.
    fn selectable<'a>(
        &self,
        id: StepId,
        line: iced::widget::Row<'a, Message>,
        section: bool,
    ) -> Element<'a, Message> {
        let p = self.palette;
        let selected = self.selected == Some(id);
        let line = if selected {
            let small = |label: &'static str, message| {
                button(text(label).size(META).color(p.primary))
                    .padding([4, 6])
                    .on_press(message)
                    .style(look::row(p, Tone::Plain))
            };
            line.push(small("↑", Message::Move(id, -1)))
                .push(small("↓", Message::Move(id, 1)))
                .push(small("✕", Message::Remove(id)))
        } else {
            line
        };
        let tone = if selected {
            Tone::Selected
        } else {
            Tone::Plain
        };
        button(line)
            .width(Fill)
            .padding(if section { [10, 10] } else { [6, 10] })
            .on_press(Message::Select(id))
            .style(look::row(p, tone))
            .into()
    }

    /// A length or budget field: what was typed, else the value, else
    /// `hint` (the length the plan uses).
    fn length_field<'a>(
        &self,
        id: StepId,
        value: Option<Duration>,
        hint: String,
        width: f32,
    ) -> Element<'a, Message> {
        let typed = self
            .drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| value.map(length::typed).unwrap_or_default());
        text_input(&hint, &typed)
            .on_input(move |text| Message::Length(id, text))
            .size(META)
            .padding([5, 8])
            .width(width)
            .style(look::input(self.palette))
            .into()
    }

    /// The launcher, adding instead of showing, with a look at the result.
    fn prepare_search(&self) -> Element<'_, Message> {
        let p = self.palette;
        let look_at: Element<'_, Message> = match &self.stage.preview {
            Some(scene) => {
                let content = self.media.preloaded(&scene.resource.id);
                self.monitor(scene, content, None)
            }
            None => crate::dashboard::empty_monitor(p, "Pick a result to look at it"),
        };
        column![
            self.search_box(),
            self.results(),
            look_at,
            text("A look only: nothing here reaches the projector.")
                .size(META)
                .color(p.on_surface_variant),
            text("Enter adds after the selected step · ↑↓ pick · Tab bang · Esc clear")
                .size(META)
                .color(p.on_surface_variant),
        ]
        .spacing(12)
        .height(Fill)
        .into()
    }

    fn estimate(&self, step: &Step) -> lantern_engine::Estimate {
        lantern_engine::timing::estimate(step, self.loaded_length(step))
    }

    /// What a section's steps add up to.
    fn planned(&self, plan: &Plan, section: StepId) -> Duration {
        let Some(start) = plan.position(section) else {
            return Duration::ZERO;
        };
        plan.steps()[start + 1..]
            .iter()
            .take_while(|s| !s.is_section())
            .map(|s| self.estimate(s).length)
            .sum()
    }
}

/// A field that reads as text until focused: no fill, no ring.
fn bare(
    p: Palette,
    color: iced::Color,
) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |theme, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        let mut style = look::input(p)(theme, status);
        if !focused {
            style.background = iced::Color::TRANSPARENT.into();
            style.border.color = iced::Color::TRANSPARENT;
        }
        style.value = color;
        style
    }
}

/// Where new steps go: a short `primary` line and its words.
fn insertion_mark(p: Palette) -> Element<'static, Message> {
    row![
        container(Space::new())
            .width(16)
            .height(2)
            .style(move |_| container::Style::default().background(p.primary)),
        text("new steps go here").size(META).color(p.primary),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .padding(iced::Padding::default().left(52.0).top(4.0).bottom(4.0))
    .into()
}

fn disc(p: Palette, icon: Icon, tone: Tone) -> Element<'static, Message> {
    let color = match tone {
        Tone::Plain => p.on_surface_variant,
        tone => tone.text(p),
    };
    container(look::icon(icon, 14.0, color))
        .width(28)
        .height(28)
        .center_x(28)
        .center_y(28)
        .style(look::chip(p, tone))
        .into()
}
