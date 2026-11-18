//! The dashboard's small parts, made once and used wherever they are
//! needed: a pill, a chip, a switch, a bar, a line between two things.
//!
//! How each is coloured is in [`look`]; what is drawn by hand (dashes,
//! hatching) is in [`crate::drawn`]; what moves is in [`crate::motion`].
//! Nothing here knows about a plan or a setting: a part takes what it
//! shows and the message it sends.

use iced::widget::{Space, button, container, row, scrollable, text, tooltip};
use iced::{Color, Element, Fill, Length};

use crate::Message;
use crate::look::{self, BODY, BOLD, MEDIUM, META, Tone};
use crate::palette::Palette;

/// A list that scrolls, with its scrollbar beside the rows instead of over them.
pub(crate) fn scrolling<'a>(
    content: impl Into<Element<'a, Message>>,
) -> scrollable::Scrollable<'a, Message> {
    scrollable(content).spacing(6).height(Fill)
}

/// One line of text; what doesn't fit is cut at the row's edge.
pub(crate) fn single_line(content: impl Into<String>) -> crate::cut::Cut {
    crate::cut::cut(content)
}

/// A few words on a tinted ground: a state, a count.
pub(crate) fn chip<'a>(p: Palette, label: &str, tone: Tone) -> Element<'a, Message> {
    container(text(label.to_string()).size(META).font(BOLD))
        .padding([3, 10])
        .style(look::chip(p, tone))
        .into()
}

/// A button with rounded ends, `height` tall. With no message it is
/// there but can't be pressed.
pub(crate) fn pill<'a>(
    p: Palette,
    label: &str,
    tone: Tone,
    height: f32,
    message: Option<Message>,
) -> button::Button<'a, Message> {
    let weight = if tone == Tone::Plain { MEDIUM } else { BOLD };
    button(
        container(text(label.to_string()).size(BODY).font(weight))
            .center_x(Length::Shrink)
            .center_y(Fill),
    )
    .height(height)
    .padding([0, 16])
    .on_press_maybe(message)
    .style(look::pill(p, tone))
}

/// `control`, saying what it does and its key once the pointer rests on
/// it. The control keeps its size: the key is beside it, not in it.
pub(crate) fn hinted<'a>(
    p: Palette,
    control: impl Into<Element<'a, Message>>,
    words: String,
    position: tooltip::Position,
) -> Element<'a, Message> {
    tooltip(
        control,
        // A long one wraps: it never runs off the window.
        container(text(words).size(META))
            .padding([4, 8])
            .max_width(380)
            .style(look::tip(p)),
        position,
    )
    .delay(look::TIP_DELAY)
    .into()
}

pub(crate) fn wide(button: button::Button<'_, Message>) -> button::Button<'_, Message> {
    button.width(Fill)
}

/// A switch, on or off.
pub(crate) fn switch<'a>(p: Palette, on: bool) -> Element<'a, Message> {
    let knob = container(Space::new().width(16).height(16)).style(move |_| container::Style {
        background: Some(
            if on {
                p.on_primary
            } else {
                p.on_surface_variant
            }
            .into(),
        ),
        border: iced::Border {
            radius: 8.0.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    });
    container(knob)
        .width(34)
        .height(20)
        .padding(2)
        .align_x(if on {
            iced::alignment::Horizontal::Right
        } else {
            iced::alignment::Horizontal::Left
        })
        .style(move |_| container::Style {
            background: Some(
                if on {
                    p.primary
                } else {
                    p.surface_container_highest
                }
                .into(),
            ),
            border: iced::Border {
                radius: 10.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// A line across, between two parts of a panel.
pub(crate) fn rule<'a>(p: Palette) -> Element<'a, Message> {
    container(Space::new().height(1))
        .width(Fill)
        .style(move |_| container::Style::default().background(p.outline_variant))
        .into()
}

/// The name over a group of things, small and in capitals.
pub(crate) fn caption<'a>(p: Palette, words: &'a str) -> iced::widget::Text<'a> {
    text(words)
        .size(11.0)
        .font(BOLD)
        .color(p.on_surface_variant)
}

/// A colour shown beside the field that sets it: `size` across, its
/// corners rounded by `radius`, with an `edge` so a dark one still shows.
pub(crate) fn swatch<'a>(
    colour: Color,
    size: f32,
    radius: f32,
    edge: Color,
) -> Element<'a, Message> {
    container(Space::new().width(size).height(size))
        .style(move |_| container::Style {
            background: Some(colour.into()),
            border: iced::Border {
                color: edge,
                width: 1.0,
                radius: radius.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// One place of a switch with several: `content` in the middle, lit
/// while it is the place in use, `tall` its padding above and below.
/// It says what it is for once the pointer rests on it.
pub(crate) fn place<'a>(
    p: Palette,
    content: impl Into<Element<'a, Message>>,
    on: bool,
    tall: u16,
    about: &str,
    message: Message,
) -> Element<'a, Message> {
    hinted(
        p,
        button(container(content).center_x(Fill))
            .width(Fill)
            .padding([tall, 0])
            .on_press(message)
            // The place in use is lit; the others are flat on the
            // switch's own ground.
            .style(move |theme, status| match on {
                true => look::pill(p, Tone::Solid)(theme, status),
                false => look::row(p, Tone::Plain)(theme, status),
            }),
        about.to_string(),
        tooltip::Position::Bottom,
    )
}

/// The ink of what is in a [`place`]: dark on the lit one.
pub(crate) fn place_ink(p: Palette, on: bool) -> Color {
    if on { p.on_primary } else { p.on_surface }
}

/// The ground a switch's places sit on, its corners rounded by `radius`:
/// one switch with several places, not several buttons.
pub(crate) fn places<'a>(
    p: Palette,
    places: impl IntoIterator<Item = Element<'a, Message>>,
    radius: f32,
) -> iced::widget::Container<'a, Message> {
    container(row(places).spacing(2))
        .padding(3)
        .style(move |_| container::Style {
            background: Some(p.surface_container_highest.into()),
            border: iced::Border {
                radius: radius.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
}

/// What a part of a [`bar`] is filled with.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Filled {
    Solid(Color),
    /// Slanted stripes of two colours: there, but not yet.
    Hatched(Color, Color),
}

/// A bar `tall` high in parts, each as wide as its share: how full
/// something is, and with what. A share is out of 1; the parts fill the
/// width between them, so they need not add up.
pub(crate) fn bar<'a>(
    parts: impl IntoIterator<Item = (f32, Filled)>,
    tall: f32,
) -> iced::widget::Row<'a, Message> {
    let wide =
        |share: f32| Length::FillPortion((share.clamp(0.0, 1.0) * 1000.0).round().max(1.0) as u16);
    row(parts.into_iter().map(|(share, filled)| match filled {
        Filled::Solid(colour) => container(Space::new())
            .width(wide(share))
            .height(tall)
            .style(move |_| container::Style::default().background(colour))
            .into(),
        Filled::Hatched(light, dark) => crate::drawn::hatched(light, dark)
            .width(wide(share))
            .height(tall)
            .into(),
    }))
}

/// How wide a time such as "12:03" is given at `size`, whatever its
/// figures are: every figure counts as wide as the widest, so what is
/// beside a clock stays where it is as the clock counts.
pub fn steady(words: &str, size: f32) -> f32 {
    words
        .chars()
        .map(|c| match c {
            '0'..='9' => 0.66,
            ':' | '.' | ' ' | '·' => 0.32,
            _ => 0.72,
        })
        .sum::<f32>()
        * size
}
