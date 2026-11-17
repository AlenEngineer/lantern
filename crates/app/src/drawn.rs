//! What a border and a background can't say: a dashed outline, for
//! something proposed and not there yet, and a hatched fill for the
//! time it would take.

use iced::widget::canvas::{self, Frame, Geometry, LineDash, Path, Stroke};
use iced::widget::{canvas as draw, stack};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};

/// A drawing kept from one frame to the next: it is made again only
/// when what it shows (`K`) or its size changes. While a video plays the
/// window is drawn many times a second, and each of these would be
/// worked out afresh every time.
pub(crate) struct Kept<K> {
    cache: canvas::Cache,
    of: std::cell::RefCell<Option<K>>,
}

impl<K> Default for Kept<K> {
    fn default() -> Self {
        Kept {
            cache: canvas::Cache::new(),
            of: std::cell::RefCell::new(None),
        }
    }
}

impl<K: PartialEq> Kept<K> {
    pub(crate) fn draw(
        &self,
        of: K,
        renderer: &Renderer,
        size: Size,
        draw: impl FnOnce(&mut Frame),
    ) -> Geometry {
        if self.of.borrow().as_ref() != Some(&of) {
            self.cache.clear();
            *self.of.borrow_mut() = Some(of);
        }
        self.cache.draw(renderer, size, draw)
    }
}

/// The dashes of an outline, and of a dot: how long each is, and the
/// space between.
const DASH: [f32; 2] = [3.0, 3.0];
const DOT_DASH: [f32; 2] = [2.0, 2.0];

/// A dashed outline with rounded corners, the size of what it is laid over.
struct Outline {
    color: Color,
    width: f32,
    radius: f32,
    dash: [f32; 2],
}

impl<Message> canvas::Program<Message> for Outline {
    type State = Kept<(Color, f32, f32, [f32; 2])>;

    fn draw(
        &self,
        kept: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        vec![kept.draw(
            (self.color, self.width, self.radius, self.dash),
            renderer,
            bounds.size(),
            |frame| {
                // Inside the edge by half its width, so none of it is cut off.
                let half = self.width / 2.0;
                let size = Size::new(
                    (bounds.width - self.width).max(0.0),
                    (bounds.height - self.width).max(0.0),
                );
                let radius = self.radius.min(size.width / 2.0).min(size.height / 2.0);
                let path = Path::rounded_rectangle(Point::new(half, half), size, radius.into());
                frame.stroke(
                    &path,
                    Stroke {
                        line_dash: LineDash {
                            segments: &self.dash,
                            offset: 0,
                        },
                        ..Stroke::default()
                            .with_color(self.color)
                            .with_width(self.width)
                    },
                );
            },
        )]
    }
}

/// `content` with a dashed outline round it.
pub fn dashed<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    color: Color,
    radius: f32,
) -> Element<'a, Message> {
    stack![
        content.into(),
        draw(Outline {
            color,
            width: 1.5,
            radius,
            dash: DASH,
        })
        .width(Fill)
        .height(Fill),
    ]
    .into()
}

/// A dashed ring: the dot of a step that is not in the plan yet.
pub fn dashed_dot<'a, Message: 'a>(size: f32, color: Color) -> Element<'a, Message> {
    draw(Outline {
        color,
        width: 1.5,
        radius: size / 2.0,
        dash: DOT_DASH,
    })
    .width(size)
    .height(size)
    .into()
}

/// Slanted stripes of two colours.
struct Hatch {
    light: Color,
    dark: Color,
}

/// How wide a stripe is, across it.
const STRIPE: f32 = 4.0;

impl<Message> canvas::Program<Message> for Hatch {
    type State = Kept<(Color, Color)>;

    fn draw(
        &self,
        kept: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        vec![
            kept.draw((self.light, self.dark), renderer, bounds.size(), |frame| {
                frame.fill_rectangle(Point::ORIGIN, bounds.size(), self.dark);
                // Each stripe leans half a right angle, so along the bar one
                // light and one dark take this much.
                let step = STRIPE * 2.0 * std::f32::consts::SQRT_2;
                let tall = bounds.height;
                let mut x = -tall - step;
                while x < bounds.width + step {
                    // Past both edges, so its square ends are out of sight.
                    let line = Path::line(
                        Point::new(x - STRIPE, tall + STRIPE),
                        Point::new(x + tall + STRIPE, -STRIPE),
                    );
                    frame.stroke(
                        &line,
                        Stroke::default().with_color(self.light).with_width(STRIPE),
                    );
                    x += step;
                }
            }),
        ]
    }
}

/// A hatched fill, as wide and tall as it is given room for.
pub fn hatched<'a, Message: 'a>(
    light: Color,
    dark: Color,
) -> iced::widget::Canvas<impl canvas::Program<Message> + 'a, Message> {
    draw(Hatch { light, dark })
}
