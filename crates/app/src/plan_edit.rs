//! Editing the plan in place, in the plan pane: the selected step opens a
//! strip with its name, its length (or a section's time, or a live slot's
//! search) and buttons to move or remove it, and new steps go right after
//! it.

use std::time::Duration;

use iced::widget::{Space, button, container, row, text, text_input, tooltip};
use iced::{Alignment, Element, Fill};
use lantern_engine::{Action, Kind, Plan, Step, StepId};

use crate::keys::{self, Op};
use crate::look::{self, BODY, Icon, META, Tone};
use crate::palette::Palette;
use crate::{App, Message, length};

const LENGTH_FIELD: f32 = 56.0;

/// A step the Add buttons make.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewStep {
    Section,
    LiveSlot,
    BlackScreen,
    Note,
}

impl NewStep {
    /// The kinds, in the order the Add step row offers them.
    pub const ALL: [(NewStep, &'static str); 4] = [
        (NewStep::Section, "Section"),
        (NewStep::LiveSlot, "Live slot"),
        (NewStep::BlackScreen, "Black"),
        (NewStep::Note, "Note"),
    ];

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

/// What of a step is being edited, in its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Name,
    /// A step's length, or a section's time.
    Length,
    /// What a live slot searches.
    Search,
}

/// The widget id of the field that edits `part` of step `id`, to focus it.
pub fn part_id(id: StepId, part: Part) -> String {
    format!("step-{id}-{part:?}")
}

impl App {
    /// Whether `part` of step `id` is being edited.
    pub(crate) fn editing(&self, id: StepId, part: Part) -> bool {
        self.editing == Some((id, part))
    }

    /// A step's name as a field, in place of its title. Enter is done.
    pub(crate) fn name_field(&self, step: &Step) -> Element<'_, Message> {
        let id = step.id;
        text_input("Name", &step.label)
            .id(part_id(id, Part::Name))
            .on_input(move |label| Message::Rename(id, label))
            .on_submit(Message::EditStep(None))
            .size(BODY)
            .padding([3, 6])
            .style(look::input(self.palette))
            .into()
    }

    /// What a live slot searches as a field, in place of its second line.
    pub(crate) fn search_field(&self, step: &Step) -> Element<'_, Message> {
        let id = step.id;
        let typed = match &step.action {
            Action::Live { hint, .. } => hint.as_ref().map(|q| q.text.as_str()),
            _ => None,
        };
        text_input("searches… (!yt, !b)", typed.unwrap_or_default())
            .id(part_id(id, Part::Search))
            .on_input(move |text| Message::SlotSearch(id, text))
            .on_submit(Message::EditStep(None))
            .size(META)
            .padding([3, 6])
            .style(look::input(self.palette))
            .into()
    }

    /// A step's length as a field, in place of the length in its row. It
    /// suggests the length the plan uses, or says what goes in it.
    pub(crate) fn step_length_field(&self, step: &Step) -> Element<'_, Message> {
        let estimate = self.estimate(step);
        let hint = match estimate.basis {
            lantern_engine::Basis::Guess if estimate.length.is_zero() => "min".into(),
            _ => length::typed(estimate.length),
        };
        self.length_field(step.id, step.estimate, hint)
    }

    /// A section's time as a field, in place of its time in its row.
    pub(crate) fn section_time_field(
        &self,
        id: StepId,
        budget: Option<Duration>,
    ) -> Element<'_, Message> {
        let p = self.palette;
        row![self.length_field(id, budget, "15".into()), note(p, "min")]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    }

    /// What can be done to the selected step, small, for its row: what a
    /// live slot searches, going on by itself, rename, move and remove.
    pub(crate) fn step_actions(&self, step: &Step) -> Element<'_, Message> {
        let p = self.palette;
        let id = step.id;
        let tip = |about: String| {
            container(text(about).size(META))
                .padding([4, 8])
                .style(look::tip(p))
        };
        let small = |content: Element<'static, Message>, tone, about: String, message| {
            tooltip(
                button(content)
                    .padding([2, 6])
                    .on_press(message)
                    .style(look::row(p, tone)),
                tip(about),
                tooltip::Position::Top,
            )
        };
        let sign = |label: &'static str| -> Element<'static, Message> {
            text(label).size(META).color(p.primary).into()
        };
        let mut actions = row![].spacing(2).align_y(Alignment::Center);
        if matches!(step.action, Action::Live { .. }) {
            actions = actions.push(small(
                look::icon(Icon::Search, 13.0, p.primary).into(),
                Tone::Plain,
                "What it searches when you get there".into(),
                Message::EditStep(Some((id, Part::Search))),
            ));
        }
        if let Some(when) = goes_on_when(step) {
            let on = step.advance;
            actions = actions.push(small(
                text("auto")
                    .size(META)
                    .color(if on { p.primary } else { p.on_surface_variant })
                    .into(),
                if on { Tone::Selected } else { Tone::Plain },
                format!(
                    "Go on when done ({when}): {}",
                    if on { "on" } else { "off" }
                ),
                Message::SetAdvance(id, !on),
            ));
        }
        actions
            .push(small(
                look::icon(Icon::Pencil, 13.0, p.primary).into(),
                Tone::Plain,
                keys::with_key("Rename", Op::Rename),
                Message::EditStep(Some((id, Part::Name))),
            ))
            .push(small(
                sign("↑"),
                Tone::Plain,
                keys::with_key("Move up", Op::MoveUp),
                Message::Move(id, -1),
            ))
            .push(small(
                sign("↓"),
                Tone::Plain,
                keys::with_key("Move down", Op::MoveDown),
                Message::Move(id, 1),
            ))
            .push(small(
                sign("✕"),
                Tone::Plain,
                keys::with_key("Remove", Op::RemoveStep),
                Message::Remove(id),
            ))
            .into()
    }

    /// A length or time field: what was typed, else the value, else `hint`
    /// (the length the plan uses).
    fn length_field<'a>(
        &self,
        id: StepId,
        value: Option<Duration>,
        hint: String,
    ) -> Element<'a, Message> {
        let typed = self
            .drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| value.map(length::typed).unwrap_or_default());
        text_input(&hint, &typed)
            .id(part_id(id, Part::Length))
            .on_input(move |text| Message::Length(id, text))
            .on_submit(Message::EditStep(None))
            .size(META)
            .padding([3, 6])
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

/// Where Ctrl+A puts the result: a `primary` line across the plan, between
/// its rows. Drawn only while a search has a result to add.
pub fn insertion_mark(p: Palette, indent: f32) -> Element<'static, Message> {
    row![
        container(Space::new())
            .width(Fill)
            .height(2)
            .style(move |_| container::Style::default().background(p.primary)),
        text(format!("{} adds here", keys::label(Op::AddToPlan)))
            .size(META)
            .color(p.primary),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .padding(
        iced::Padding::default()
            .left(indent + 10.0)
            .right(10.0)
            .top(2.0)
            .bottom(2.0),
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
