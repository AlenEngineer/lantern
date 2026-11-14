//! How the dashboard looks: fonts, icons, and one style function per kind
//! of control, all from [`Palette`] roles. `docs/DESIGN.md` explains the
//! rules these follow.

use iced::widget::{button, container, slider, svg, text_input};
use iced::{Background, Border, Color, Font, font};

use crate::palette::Palette;

pub const UI: Font = Font::with_name("Inter");
pub const MEDIUM: Font = Font {
    weight: font::Weight::Medium,
    ..UI
};
pub const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..UI
};

/// Text sizes, by purpose.
pub const HEADING: f32 = 16.0;
pub const BODY: f32 = 13.0;
pub const META: f32 = 12.0;

/// Row and panel corners.
pub const ROW_RADIUS: f32 = 12.0;
/// Buttons and chips are pills: iced caps a radius at half the height.
pub const PILL_RADIUS: f32 = 999.0;
/// Standard and compact pill heights.
pub const PILL: f32 = 36.0;
pub const PILL_SMALL: f32 = 30.0;

/// The hover veil: `on_surface` at 12% over whatever is under it.
const VEIL: f32 = 0.12;
/// A selected or current item: its role at 22%.
const SELECTED: f32 = 0.22;

/// What a row or button stands for, which decides its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Routine: no fill until hovered (rows), `on_surface` at 8% (buttons).
    Plain,
    /// Selected or the next thing to happen: the `primary` tint.
    Selected,
    /// On stage now: the `tertiary` tint.
    Live,
    /// Blackout and other actions that take something away: `error` at 14%.
    Danger,
    /// The one solid control, play/pause: `primary` with `on_primary`.
    Solid,
    /// Something is off that the audience notices, and this puts it
    /// back: solid `error`, to be seen across a dark booth.
    Alarm,
}

impl Tone {
    /// (fill, text) at rest.
    fn colors(self, p: Palette) -> (Color, Color) {
        match self {
            Tone::Plain => (Palette::tint(p.on_surface, 0.08), p.on_surface),
            Tone::Selected => (Palette::tint(p.primary, SELECTED), p.primary),
            Tone::Live => (Palette::tint(p.tertiary, SELECTED), p.tertiary),
            Tone::Danger => (Palette::tint(p.error, 0.14), p.error),
            Tone::Solid => (p.primary, p.on_primary),
            Tone::Alarm => (p.error, p.surface),
        }
    }

    /// The text colour, for labels inside a row of this tone.
    pub fn text(self, p: Palette) -> Color {
        self.colors(p).1
    }
}

/// A pill button of `tone`.
pub fn pill(p: Palette, tone: Tone) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        let (fill, text) = tone.colors(p);
        styled(p, fill, text, PILL_RADIUS, status)
    }
}

/// A list row: flat until hovered, tinted when it is `tone`.
pub fn row(p: Palette, tone: Tone) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_, status| {
        let fill = match tone {
            Tone::Plain => Color::TRANSPARENT,
            tone => tone.colors(p).0,
        };
        styled(p, fill, p.on_surface, ROW_RADIUS, status)
    }
}

fn styled(
    p: Palette,
    fill: Color,
    text: Color,
    radius: f32,
    status: button::Status,
) -> button::Style {
    let (background, text) = match status {
        button::Status::Hovered | button::Status::Pressed => (veil(fill, p), text),
        button::Status::Disabled => (Palette::tint(fill, fill.a * 0.5), Palette::tint(text, 0.38)),
        button::Status::Active => (fill, text),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color: text,
        border: Border {
            radius: radius.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// `on_surface` at 12% laid over `under`.
fn veil(under: Color, p: Palette) -> Color {
    let mix = |a: f32, b: f32| a * (1.0 - VEIL) + b * VEIL;
    let over = p.on_surface;
    if under.a == 0.0 {
        return Palette::tint(over, VEIL);
    }
    Color {
        r: mix(under.r, over.r),
        g: mix(under.g, over.g),
        b: mix(under.b, over.b),
        a: under.a + VEIL * (1.0 - under.a),
    }
}

/// A small pill label (a mode, a scope, a state) in `tone`.
/// A thumbnail's frame: slightly rounded, on the darkest container.
pub fn thumb(p: Palette) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(p.surface_container_highest)),
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn chip(p: Palette, tone: Tone) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| {
        let (fill, text) = tone.colors(p);
        container::Style {
            background: Some(Background::Color(fill)),
            text_color: Some(text),
            border: Border {
                radius: PILL_RADIUS.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }
    }
}

/// A tooltip: solid, so it reads over whatever it covers.
pub fn tip(p: Palette) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(p.surface_container_highest)),
        text_color: Some(p.on_surface),
        border: Border {
            radius: PILL_RADIUS.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The window: the `surface` role.
/// How round a card's corners are.
pub const CARD_RADIUS: f32 = 22.0;

/// `colour` at `share` of the way to black.
fn darker(colour: Color, share: f32) -> Color {
    Color {
        r: colour.r * (1.0 - share),
        g: colour.g * (1.0 - share),
        b: colour.b * (1.0 - share),
        a: colour.a,
    }
}

/// A card on the window's ground: the strip, the plan, the stage.
/// `surface`, less see-through than the ground it sits on so what's
/// written on it reads, with a hairline of light and a shadow to lift it.
pub fn card(
    p: Palette,
    see_through: f32,
    radius: f32,
) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(float_colour(p, see_through))),
        text_color: Some(p.on_surface),
        border: Border {
            color: Color::WHITE.scale_alpha(0.10),
            width: 1.0,
            radius: radius.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.35),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

/// The colour of the strip and of what drops from it: a card's own. A
/// shadow, not a colour, sets a dropped panel off from the card under it.
pub fn float_colour(p: Palette, see_through: f32) -> Color {
    Color {
        // Never so clear that what's written on it is lost.
        a: (1.0 - 1.15 * see_through).clamp(0.35, 1.0),
        ..p.surface
    }
}

/// The strip: a card's colour, a pill, lifted by a shadow. It has no
/// outline, so a panel can drop from it without a seam.
pub fn strip(p: Palette, see_through: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(float_colour(p, see_through))),
        text_color: Some(p.on_surface),
        border: Border {
            radius: PILL_RADIUS.into(),
            ..Border::default()
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.35),
            offset: iced::Vector::new(0.0, 6.0),
            blur_radius: 18.0,
        },
        ..container::Style::default()
    }
}

/// A panel that drops from the strip: the strip's own colour and no edge
/// where they meet, so the two read as one body. Square at the top, where
/// its [shoulders](shoulder) flare into the strip.
pub fn panel(p: Palette, see_through: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(float_colour(p, see_through))),
        text_color: Some(p.on_surface),
        border: Border {
            radius: iced::border::Radius::default()
                .bottom_left(CARD_RADIUS)
                .bottom_right(CARD_RADIUS),
            ..Border::default()
        },
        // Thrown well down, so little of it reaches up to the strip,
        // which is drawn over it anyway.
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.55),
            offset: iced::Vector::new(0.0, 16.0),
            blur_radius: 28.0,
        },
        ..container::Style::default()
    }
}

/// How far a dropped panel's shoulders reach to each side of it.
pub const SHOULDER: f32 = 18.0;

/// One shoulder of a dropped panel: the curve that carries the strip's
/// underside down into the panel's side, the way a drop hangs from the
/// water it's leaving. `left` is the one on the panel's left. Coloured as
/// the panel is.
pub fn shoulder<'a>(left: bool, color: Color) -> svg::Svg<'a> {
    // A square with a quarter disc taken out of the corner away from
    // the panel: what's left is the fillet between strip and panel.
    const LEFT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0H24V24A24 24 0 0 0 0 0Z"/></svg>"#;
    const RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M24 0H0V24A24 24 0 0 1 24 0Z"/></svg>"#;
    svg(svg::Handle::from_memory(if left { LEFT } else { RIGHT }))
        .width(SHOULDER)
        .height(SHOULDER)
        .style(move |_, _| svg::Style { color: Some(color) })
}

/// An item of the strip: a pill with no fill until it's hovered or lit.
pub fn strip_item(
    p: Palette,
    tone: Tone,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |theme, status| match tone {
        Tone::Plain => {
            let mut style = row(p, Tone::Plain)(theme, status);
            style.border.radius = PILL_RADIUS.into();
            style
        }
        tone => pill(p, tone)(theme, status),
    }
}

/// The window's ground, under its cards: darker than `surface`, and
/// `see_through` of the way to clear (0 is solid), so the desktop shows
/// through a transparent window.
pub fn window(p: Palette, see_through: f32) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(Color {
            a: (1.0 - 1.2 * see_through).clamp(0.06, 1.0),
            ..darker(p.surface, 0.35)
        })),
        text_color: Some(p.on_surface),
        ..container::Style::default()
    }
}

/// A picture of the stage: black, ringed in `ring` (none for an empty one).
pub fn monitor(ring: Option<Color>) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(Color::BLACK)),
        border: Border {
            color: ring.unwrap_or(Color::TRANSPARENT),
            width: if ring.is_some() { 2.0 } else { 0.0 },
            radius: ROW_RADIUS.into(),
        },
        ..container::Style::default()
    }
}

/// The search field: `surface_container`, ringed in `primary` while focused.
pub fn input(p: Palette) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style {
    move |_, status| {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: Background::Color(p.surface_container),
            border: Border {
                color: if focused {
                    p.primary
                } else {
                    Color::TRANSPARENT
                },
                width: 2.0,
                radius: ROW_RADIUS.into(),
            },
            icon: p.on_surface_variant,
            // Dimmer than anything typed, so an example isn't taken for a
            // value that's already there.
            placeholder: p.outline,
            value: p.on_surface,
            selection: Palette::tint(p.primary, 0.4),
        }
    }
}

/// The timeline: a `surface_container_highest` track filled with `primary`.
pub fn timeline(p: Palette) -> impl Fn(&iced::Theme, slider::Status) -> slider::Style {
    move |_, _| slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(p.primary),
                Background::Color(p.surface_container_highest),
            ),
            width: 4.0,
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: 7.0 },
            background: Background::Color(p.primary),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

/// An icon in `color`.
pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> svg::Svg<'a> {
    svg(svg::Handle::from_memory(icon.svg()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
}

/// The icons the dashboard uses, from Material Design Icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Search,
    Play,
    Pause,
    Replay,
    Video,
    Image,
    Music,
    Document,
    Book,
    Link,
    Settings,
    Folder,
    Clock,
    Pencil,
}

impl Icon {
    fn svg(self) -> &'static [u8] {
        macro_rules! mdi {
            ($path:literal) => {
                concat!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d=""#,
                    $path,
                    r#""/></svg>"#
                )
                .as_bytes()
            };
        }
        match self {
            Icon::Search => mdi!(
                "M9.5,3A6.5,6.5 0 0,1 16,9.5C16,11.11 15.41,12.59 14.44,13.73L14.71,14H15.5L20.5,19L19,20.5L14,15.5V14.71L13.73,14.44C12.59,15.41 11.11,16 9.5,16A6.5,6.5 0 0,1 3,9.5A6.5,6.5 0 0,1 9.5,3M9.5,5C7,5 5,7 5,9.5C5,12 7,14 9.5,14C12,14 14,12 14,9.5C14,7 12,5 9.5,5Z"
            ),
            Icon::Play => mdi!("M8,5.14V19.14L19,12.14L8,5.14Z"),
            Icon::Pause => mdi!("M14,19H18V5H14M6,19H10V5H6V19Z"),
            Icon::Replay => mdi!(
                "M12,5V1L7,6L12,11V7A6,6 0 0,1 18,13A6,6 0 0,1 12,19A6,6 0 0,1 6,13H4A8,8 0 0,0 12,21A8,8 0 0,0 20,13A8,8 0 0,0 12,5Z"
            ),
            Icon::Video => mdi!(
                "M17,10.5V7A1,1 0 0,0 16,6H4A1,1 0 0,0 3,7V17A1,1 0 0,0 4,18H16A1,1 0 0,0 17,17V13.5L21,17.5V6.5L17,10.5Z"
            ),
            Icon::Image => mdi!(
                "M8.5,13.5L11,16.5L14.5,12L19,18H5M21,19V5C21,3.89 20.1,3 19,3H5A2,2 0 0,0 3,5V19A2,2 0 0,0 5,21H19A2,2 0 0,0 21,19Z"
            ),
            Icon::Music => mdi!(
                "M21,3V15.5A3.5,3.5 0 0,1 17.5,19A3.5,3.5 0 0,1 14,15.5A3.5,3.5 0 0,1 17.5,12C18.04,12 18.55,12.12 19,12.34V6.47L9,8.6V17.5A3.5,3.5 0 0,1 5.5,21A3.5,3.5 0 0,1 2,17.5A3.5,3.5 0 0,1 5.5,14C6.04,14 6.55,14.12 7,14.34V6L21,3Z"
            ),
            Icon::Document => mdi!(
                "M13,9H18.5L13,3.5V9M6,2H14L20,8V20A2,2 0 0,1 18,22H6C4.89,22 4,21.1 4,20V4C4,2.89 4.89,2 6,2M15,18V16H6V18H15M18,14V12H6V14H18Z"
            ),
            Icon::Book => mdi!(
                "M18,22A2,2 0 0,0 20,20V4C20,2.89 19.1,2 18,2H12V9L9.5,7.5L7,9V2H6A2,2 0 0,0 4,4V20A2,2 0 0,0 6,22H18Z"
            ),
            Icon::Settings => mdi!(
                "M12,15.5A3.5,3.5 0 0,1 8.5,12A3.5,3.5 0 0,1 12,8.5A3.5,3.5 0 0,1 15.5,12A3.5,3.5 0 0,1 12,15.5M19.43,12.97C19.47,12.65 19.5,12.33 19.5,12C19.5,11.67 19.47,11.34 19.43,11L21.54,9.37C21.73,9.22 21.78,8.95 21.66,8.73L19.66,5.27C19.54,5.05 19.27,4.96 19.05,5.05L16.56,6.05C16.04,5.66 15.5,5.32 14.87,5.07L14.5,2.42C14.46,2.18 14.25,2 14,2H10C9.75,2 9.54,2.18 9.5,2.42L9.13,5.07C8.5,5.32 7.96,5.66 7.44,6.05L4.95,5.05C4.73,4.96 4.46,5.05 4.34,5.27L2.34,8.73C2.21,8.95 2.27,9.22 2.46,9.37L4.57,11C4.53,11.34 4.5,11.67 4.5,12C4.5,12.33 4.53,12.65 4.57,12.97L2.46,14.63C2.27,14.78 2.21,15.05 2.34,15.27L4.34,18.73C4.46,18.95 4.73,19.03 4.95,18.95L7.44,17.94C7.96,18.34 8.5,18.68 9.13,18.93L9.5,21.58C9.54,21.82 9.75,22 10,22H14C14.25,22 14.46,21.82 14.5,21.58L14.87,18.93C15.5,18.67 16.04,18.34 16.56,17.94L19.05,18.95C19.27,19.03 19.54,18.95 19.66,18.73L21.66,15.27C21.78,15.05 21.73,14.78 21.54,14.63L19.43,12.97Z"
            ),
            Icon::Pencil => mdi!(
                "M20.71,7.04C21.1,6.65 21.1,6 20.71,5.63L18.37,3.29C18,2.9 17.35,2.9 16.96,3.29L15.12,5.12L18.87,8.87M3,17.25V21H6.75L17.81,9.93L14.06,6.18L3,17.25Z"
            ),
            Icon::Clock => mdi!(
                "M12,20A8,8 0 0,0 20,12A8,8 0 0,0 12,4A8,8 0 0,0 4,12A8,8 0 0,0 12,20M12,2A10,10 0 0,1 22,12A10,10 0 0,1 12,22C6.47,22 2,17.5 2,12A10,10 0 0,1 12,2M12.5,7V12.25L17,14.92L16.25,16.15L11,13V7H12.5Z"
            ),
            Icon::Folder => mdi!(
                "M10,4H4C2.89,4 2,4.89 2,6V18A2,2 0 0,0 4,20H20A2,2 0 0,0 22,18V8C22,6.89 21.1,6 20,6H12L10,4Z"
            ),
            Icon::Link => mdi!(
                "M3.9,12C3.9,10.29 5.29,8.9 7,8.9H11V7H7A5,5 0 0,0 2,12A5,5 0 0,0 7,17H11V15.1H7C5.29,15.1 3.9,13.71 3.9,12M8,13H16V11H8V13M17,7H13V8.9H17C18.71,8.9 20.1,10.29 20.1,12C20.1,13.71 18.71,15.1 17,15.1H13V17H17A5,5 0 0,0 22,12A5,5 0 0,0 17,7Z"
            ),
        }
    }

    /// The icon for a kind of resource.
    pub fn of(kind: &lantern_engine::Kind) -> Icon {
        use lantern_engine::Kind;
        match kind {
            Kind::Video => Icon::Video,
            Kind::Image => Icon::Image,
            Kind::Audio => Icon::Music,
            Kind::Text => Icon::Book,
            Kind::Web => Icon::Link,
            _ => Icon::Document,
        }
    }
}

/// The iced theme, so built-in widgets (scrollbars) match.
pub fn theme(p: Palette) -> iced::Theme {
    iced::Theme::custom(
        "lantern",
        iced::theme::Palette {
            background: p.surface,
            text: p.on_surface,
            primary: p.primary,
            success: p.tertiary,
            warning: p.tertiary,
            danger: p.error,
        },
    )
}
