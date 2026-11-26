//! Notes for the operator on what they are about: a small numbered mark
//! where each is, a card to read and write one, and in a service what
//! is noted of the step that is on, and of the one next.

use iced::widget::{Space, button, column, container, mouse_area, row, text, text_editor};
use iced::{Alignment, Color, Element, Fill, mouse};
use lantern_engine::{NoteId, NoteOn, Plan, StepId};

use crate::look::{self, BOLD, Icon, META};
use crate::palette::Palette;
use crate::{App, Message};

/// The colour of a note, whatever the look: a note stands apart from
/// what it is on.
pub const INK: Color = Color::from_rgb(0.953, 0.835, 0.541);

/// How wide a note's card is.
pub const CARD: f32 = 190.0;

/// A note's mark, numbered `n`: pressed, the note opens.
pub fn mark<'a>(n: usize, open: Option<Message>) -> Element<'a, Message> {
    let face = container(
        text(n.to_string())
            .size(9)
            .font(BOLD)
            .color(Color::from_rgb(0.09, 0.09, 0.11)),
    )
    .center_x(13)
    .center_y(13)
    .style(|_| container::Style {
        background: Some(INK.into()),
        border: iced::Border {
            radius: 3.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    });
    match open {
        Some(open) => mouse_area(face)
            .on_press(open)
            .interaction(mouse::Interaction::Pointer)
            .into(),
        None => face.into(),
    }
}

/// The notes on what is noted of the step `id`, as they are numbered:
/// its own, and with `around` those of the sections round it and of the
/// loops and drawn orders it is in.
///
/// A section's notes come only with the first of its steps to be reached:
/// `reached` says which the service has reached.
pub fn on_step(
    plan: &Plan,
    id: StepId,
    around: bool,
    reached: &dyn Fn(StepId) -> bool,
) -> Vec<(NoteId, usize, String)> {
    let numbers = plan.numbered_notes();
    let number = |note: NoteId| {
        numbers
            .iter()
            .find(|(id, _)| *id == note)
            .map_or(0, |(_, n)| *n)
    };
    let mut on = vec![NoteOn::Step(id)];
    if around && let Some(at) = plan.position(id) {
        let steps = plan.steps();
        let first_in = |section: StepId| {
            plan.position(section).is_some_and(|head| {
                (head + 1..plan.group_end(head))
                    .map(|row| &steps[row])
                    .filter(|step| !step.is_section() && step.id != id)
                    .all(|step| !reached(step.id))
            })
        };
        on.extend(
            plan.within(at)
                .into_iter()
                .filter(|section| first_in(*section))
                .map(NoteOn::Step),
        );
        on.extend(plan.spans_at(at).into_iter().map(NoteOn::Span));
    }
    let mut found: Vec<(NoteId, usize, String)> = Vec::new();
    for at in on {
        for note in plan.notes_on(at) {
            if !found.iter().any(|(id, ..)| *id == note.id) {
                found.push((note.id, number(note.id), note.text.clone()));
            }
        }
    }
    found.sort_by_key(|(_, n, _)| *n);
    found
}

/// The notes on `on`, with what each is numbered, in that order.
pub fn numbers_on(plan: &Plan, on: NoteOn) -> Vec<(NoteId, usize)> {
    let numbers = plan.numbered_notes();
    let mut found: Vec<(NoteId, usize)> = plan
        .notes_on(on)
        .into_iter()
        .filter_map(|note| numbers.iter().find(|(id, _)| *id == note.id))
        .copied()
        .collect();
    found.sort_unstable_by_key(|(_, n)| *n);
    found
}

/// The step whose row a note on `on` is shown by: the step, a section's
/// header, or a loop's or a drawn order's first step.
pub fn anchor(plan: &Plan, on: NoteOn) -> Option<StepId> {
    match on {
        NoteOn::Step(id) => plan.get(id).map(|_| id),
        NoteOn::Span(id) => plan.span_range(id).map(|(first, _)| plan.steps()[first].id),
    }
}

/// A note as it is said in a service: its mark and what it says.
pub fn said<'a>(p: Palette, n: usize, words: String) -> Element<'a, Message> {
    container(
        row![
            mark(n, None),
            text(words).size(META).color(p.on_surface).width(Fill)
        ]
        .spacing(8)
        .align_y(Alignment::Start),
    )
    .padding([6, 8])
    .width(Fill)
    .style(move |_| container::Style {
        background: Some(Color { a: 0.10, ..INK }.into()),
        border: iced::Border {
            color: Color { a: 0.5, ..INK },
            width: 1.0,
            radius: 9.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

impl App {
    /// The open note's card: its mark, what it is on, what it says (to
    /// be written in), and the way to take it away.
    pub(crate) fn note_card<'a>(&'a self, plan: &Plan) -> Option<Element<'a, Message>> {
        let p = self.palette;
        let id = self.note_open?;
        let note = plan.note(id)?;
        let n = plan
            .numbered_notes()
            .into_iter()
            .find(|(of, _)| *of == id)
            .map_or(0, |(_, n)| n);
        let on = match note.on.len() {
            1 => match note.on[0] {
                NoteOn::Step(of) if plan.get(of).is_some_and(|step| step.is_section()) => {
                    "the section".to_string()
                }
                NoteOn::Step(_) => "1 step".to_string(),
                NoteOn::Span(of) if plan.span(of).is_some_and(|span| span.is_loop()) => {
                    "the loop".to_string()
                }
                NoteOn::Span(_) => "the drawn order".to_string(),
            },
            more if note.on.iter().all(|on| {
                matches!(on, NoteOn::Step(id) if plan.get(*id).is_some_and(|step| !step.is_section()))
            }) =>
            {
                format!("{more} steps")
            }
            more => format!("{more} things"),
        };
        let head = row![
            mark(n, None),
            text(on).size(META).font(BOLD).color(INK),
            Space::new().width(Fill),
            button(look::icon(Icon::Close, 12.0, p.on_surface_variant))
                .padding([2, 4])
                .on_press(Message::NoteGone(id))
                .style(look::row(p, look::Tone::Plain)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let words = text_editor(&self.note_text)
            .id(NOTE_FIELD)
            .placeholder("What to remember…")
            .on_action(Message::NoteEdit)
            .size(META)
            .padding(6)
            .style(move |theme, status| {
                let mut style = text_editor::default(theme, status);
                style.background = Color::TRANSPARENT.into();
                style.border = iced::Border::default();
                style.value = p.on_surface;
                style.placeholder = p.on_surface_variant;
                style
            });
        Some(
            container(column![head, words].spacing(4))
                .width(CARD)
                .padding(8)
                .style(move |_| container::Style {
                    background: Some(Color::from_rgb(0.17, 0.165, 0.13).into()),
                    border: iced::Border {
                        color: Color { a: 0.6, ..INK },
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    shadow: iced::Shadow {
                        color: Color::BLACK.scale_alpha(0.4),
                        offset: iced::Vector::new(0.0, 4.0),
                        blur_radius: 12.0,
                    },
                    ..container::Style::default()
                })
                .into(),
        )
    }
}

/// The field a note is written in.
pub const NOTE_FIELD: &str = "note-words";
