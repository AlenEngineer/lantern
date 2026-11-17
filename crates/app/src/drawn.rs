//! What a border and a background can't say: a dashed outline, for
//! something proposed and not there yet, and a hatched fill for the
//! time it would take.

use iced::widget::canvas::{self, Frame, Geometry, LineDash, Path, Stroke};
use iced::widget::{canvas as draw, stack};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};

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
    type State = ();

    fn draw(
        &self,
        (): &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
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
        vec![frame.into_geometry()]
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

/// The line that curls back beside the steps of something that comes
/// again, the piece of it beside one step: drawn the whole height of the
/// step's row, in dashes spaced to fit that height exactly, so that one
/// row's piece meets the next with no seam.
struct CurlPiece {
    piece: crate::look::Curl,
    color: Color,
    /// How far down the row the step's dot is.
    dot: f32,
}

/// Where the line runs down, the dashes' length, about how far from one
/// to the next, and how wide the bend to the dot is.
const CURL_X: f32 = 3.0;
const CURL_DASH: f32 = 2.5;
const CURL_STEP: f32 = 5.0;
const CURL_BEND: f32 = 4.0;

impl<Message> canvas::Program<Message> for CurlPiece {
    type State = ();

    fn draw(
        &self,
        (): &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        use crate::look::Curl;
        let mut frame = Frame::new(renderer, bounds.size());
        let stroke = Stroke {
            line_cap: canvas::LineCap::Round,
            line_join: canvas::LineJoin::Round,
            ..Stroke::default().with_color(self.color).with_width(1.5)
        };
        // Dashes down from `from` to `to`, as many as fit, each in the
        // middle of its share: half a gap at either end.
        let mut down = |from: f32, to: f32| {
            let tall = to - from;
            if tall < CURL_DASH {
                return;
            }
            let count = (tall / CURL_STEP).round().max(1.0);
            let share = tall / count;
            for n in 0..count as usize {
                let top = from + share * n as f32 + (share - CURL_DASH) / 2.0;
                frame.stroke(
                    &Path::line(Point::new(CURL_X, top), Point::new(CURL_X, top + CURL_DASH)),
                    stroke,
                );
            }
        };
        let (tall, wide, dot) = (bounds.height, bounds.width, self.dot);
        // The bend between the line and the dot's level, one stroke.
        let bend = |down: bool| {
            let sign = if down { 1.0 } else { -1.0 };
            Path::new(|path| {
                path.move_to(Point::new(CURL_X, dot + sign * CURL_BEND));
                path.quadratic_curve_to(
                    Point::new(CURL_X, dot),
                    Point::new(CURL_X + CURL_BEND, dot),
                );
            })
        };
        match self.piece {
            Curl::Past => down(0.0, tall),
            // In to the dot, with the arrow's head, from the line below.
            Curl::Into => {
                down(dot + CURL_BEND, tall);
                frame.stroke(&bend(true), stroke);
                let head = Path::new(|path| {
                    path.move_to(Point::new(wide - 4.5, dot - 3.0));
                    path.line_to(Point::new(wide - 1.0, dot));
                    path.line_to(Point::new(wide - 4.5, dot + 3.0));
                });
                frame.stroke(&head, stroke);
            }
            // Down the line and out to the dot.
            Curl::Out => {
                down(0.0, dot - CURL_BEND);
                frame.stroke(&bend(false), stroke);
                frame.stroke(
                    &Path::line(
                        Point::new(CURL_X + CURL_BEND + 2.0, dot),
                        Point::new(wide - 0.5, dot),
                    ),
                    stroke,
                );
            }
        }
        vec![frame.into_geometry()]
    }
}

/// The piece of the curl beside one step, the whole height it is given;
/// `dot` is how far down the step's dot is.
pub fn curl<'a, Message: 'a>(
    piece: crate::look::Curl,
    color: Color,
    dot: f32,
) -> iced::widget::Canvas<impl canvas::Program<Message> + 'a, Message> {
    draw(CurlPiece { piece, color, dot })
        .width(crate::look::CURL_WIDE)
        .height(Fill)
}

/// Slanted stripes of two colours.
struct Hatch {
    light: Color,
    dark: Color,
}

/// How wide a stripe is, across it.
const STRIPE: f32 = 4.0;

impl<Message> canvas::Program<Message> for Hatch {
    type State = ();

    fn draw(
        &self,
        (): &(),
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
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
        vec![frame.into_geometry()]
    }
}

/// A hatched fill, as wide and tall as it is given room for.
pub fn hatched<'a, Message: 'a>(
    light: Color,
    dark: Color,
) -> iced::widget::Canvas<impl canvas::Program<Message> + 'a, Message> {
    draw(Hatch { light, dark })
}
