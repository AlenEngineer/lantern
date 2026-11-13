//! Editing the plan in place, in the plan pane: the selected step opens a
//! strip with its name, its length (or a section's time, or a live slot's
//! search) and buttons to move or remove it, and new steps go right after
//! it.

use std::time::Duration;

use iced::widget::{Space, button, column, container, row, text, text_input, tooltip};
use iced::{Alignment, Element, Fill};
use lantern_engine::{Action, Kind, Plan, Step, StepId};

use crate::look::{self, BODY, META, Tone};
use crate::palette::Palette;
use crate::{App, Message, length};

const LENGTH_FIELD: f32 = 64.0;

/// A step the Add buttons make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewStep {
    Section,
    LiveSlot,
    BlackScreen,
    Note,
}

impl NewStep {
    /// Its label and action, ready to rename. A section starts without a
    /// time; its row offers to set one.
    pub fn step(self) -> (&'static str, Action) {
        match self {
            NewStep::Section => ("New section", Action::Section { budget: None }),
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

/// The widget id of a step's name field, to focus it on adding.
pub fn name_id(id: StepId) -> String {
    format!("step-name-{id}")
}

/// The widget id of a section's time field.
pub fn time_id(id: StepId) -> String {
    format!("section-time-{id}")
}

impl App {
    /// The selected step's strip: its name, its length, a section's time or
    /// a live slot's search, and ↑ ↓ ✕.
    pub(crate) fn edit_strip(&self, step: &Step) -> Element<'_, Message> {
        let p = self.palette;
        let id = step.id;
        let name = text_input("Name", &step.label)
            .id(name_id(id))
            .on_input(move |label| Message::Rename(id, label))
            .size(BODY)
            .padding([6, 10])
            .style(look::input(p));
        // The name has the strip's first line to itself: beside the length
        // and the buttons it showed about eighteen characters.
        let mut line = row![].spacing(6).align_y(Alignment::Center);
        line = match &step.action {
            Action::Section { budget } => line
                .push(note(p, "time"))
                .push(self.length_field(id, *budget, "15".into(), time_id(id)))
                .push(note(p, "min")),
            Action::Live { hint, .. } => {
                let typed = hint.as_ref().map(|q| q.text.as_str()).unwrap_or_default();
                line.push(
                    text_input("searches… (!yt, !b)", typed)
                        .on_input(move |text| Message::SlotSearch(id, text))
                        .size(META)
                        .padding([6, 8])
                        .width(Fill)
                        .style(look::input(p)),
                )
            }
            _ => {
                let estimate = self.estimate(step);
                // Without a length to suggest, the empty box says what goes in it.
                let hint = match estimate.basis {
                    lantern_engine::Basis::Guess if estimate.length.is_zero() => "min".into(),
                    _ => length::typed(estimate.length),
                };
                line.push(self.length_field(id, step.estimate, hint, format!("step-length-{id}")))
            }
        };
        let small = |label: &'static str, about: &'static str, message| {
            tooltip(
                button(text(label).size(META).color(p.primary))
                    .padding([6, 8])
                    .on_press(message)
                    .style(look::row(p, Tone::Plain)),
                container(text(about).size(META).color(p.on_surface))
                    .padding([4, 8])
                    .style(look::tip(p)),
                tooltip::Position::Top,
            )
        };
        // Going on by itself sits beside the length it waits for.
        if let Some(when) = goes_on_when(step) {
            let on = step.advance;
            let switch = button(
                row![
                    text(if on { "✓" } else { "○" }).size(META).color(p.primary),
                    text("Go on when done").size(META).color(if on {
                        p.primary
                    } else {
                        p.on_surface
                    }),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding([6, 10])
            .on_press(Message::SetAdvance(id, !on))
            .style(look::row(p, if on { Tone::Selected } else { Tone::Plain }));
            line = line.push(tooltip(
                switch,
                container(text(when).size(META).color(p.on_surface))
                    .padding([4, 8])
                    .style(look::tip(p)),
                tooltip::Position::Top,
            ));
        }
        // A live slot's search already fills the line.
        if !matches!(step.action, Action::Live { .. }) {
            line = line.push(Space::new().width(Fill));
        }
        line = line
            .push(small("↑", "Move up · Alt+↑", Message::Move(id, -1)))
            .push(small("↓", "Move down · Alt+↓", Message::Move(id, 1)))
            .push(small("✕", "Remove", Message::Remove(id)));
        let strip = column![name.width(Fill), line].spacing(4);
        container(strip)
            .padding(iced::Padding::default().left(24.0).right(4.0).bottom(4.0))
            .into()
    }

    /// A length or time field: what was typed, else the value, else `hint`
    /// (the length the plan uses).
    fn length_field<'a>(
        &self,
        id: StepId,
        value: Option<Duration>,
        hint: String,
        widget: String,
    ) -> Element<'a, Message> {
        let typed = self
            .drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| value.map(length::typed).unwrap_or_default());
        text_input(&hint, &typed)
            .id(widget)
            .on_input(move |text| Message::Length(id, text))
            .size(META)
            .padding([6, 8])
            .width(LENGTH_FIELD)
            .style(look::input(self.palette))
            .into()
    }

    pub(crate) fn estimate(&self, step: &Step) -> lantern_engine::Estimate {
        lantern_engine::timing::estimate(step, self.loaded_length(step))
    }

    /// What a section's steps add up to.
    pub(crate) fn planned(&self, plan: &Plan, section: StepId) -> Duration {
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

fn note<'a>(p: Palette, words: &'a str) -> Element<'a, Message> {
    text(words).size(META).color(p.on_surface_variant).into()
}

/// Where Ctrl+A puts the next step: a short `primary` line and its words.
pub fn insertion_mark(p: Palette, indent: f32) -> Element<'static, Message> {
    column![
        row![
            container(Space::new())
                .width(16)
                .height(2)
                .style(move |_| container::Style::default().background(p.primary)),
            text("Ctrl+A adds the result here")
                .size(META)
                .color(p.primary),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .padding(
        iced::Padding::default()
            .left(indent + 10.0)
            .top(2.0)
            .bottom(4.0),
    )
    .width(Fill)
    .into()
}

/// When a step that goes on by itself would: a video when it ends, a still
/// after its length. `None` for steps that can't (sections, live slots,
/// notes), which wait for the operator.
fn goes_on_when(step: &Step) -> Option<&'static str> {
    match &step.action {
        Action::Show { resource } if matches!(resource.kind, Kind::Video | Kind::Audio) => {
            Some("when it ends")
        }
        Action::Show { .. } | Action::Blank => Some("after its length"),
        Action::Section { .. } | Action::Live { .. } | Action::Note => None,
    }
}
