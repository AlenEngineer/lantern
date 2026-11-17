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
struct Kept<K> {
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
    fn draw(
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

/// Where on a piece of a line its mark sits, so that it is at the middle
/// of the whole line: level with this step's dot, when the line has a
/// step at its middle; or half way to the next step's dot or the one
/// before's, when its middle is between two. Then both pieces draw it,
/// each the part that is in its own row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkAt {
    Dot,
    Below,
    Above,
}

impl MarkAt {
    /// Which row of `rows` (the line's steps, top to bottom) carries the
    /// mark, as the row at `row` sees it.
    pub fn of(rows: usize, row: usize) -> Option<MarkAt> {
        match rows % 2 {
            1 if row == rows / 2 => Some(MarkAt::Dot),
            0 if row + 1 == rows / 2 => Some(MarkAt::Below),
            0 if row == rows / 2 => Some(MarkAt::Above),
            _ => None,
        }
    }

    /// How far down a piece `tall` high, whose dot is `dot` down, the
    /// mark's middle is.
    fn middle(self, dot: f32, tall: f32) -> f32 {
        match self {
            MarkAt::Dot => dot,
            MarkAt::Below => dot + tall / 2.0,
            MarkAt::Above => dot - tall / 2.0,
        }
    }
}

/// The parts of the run from `from` to `to` that a mark with its middle
/// at `middle` leaves: above it and below it.
fn round_mark(from: f32, to: f32, middle: f32) -> [(f32, f32); 2] {
    [
        (from, to.min(middle - MARK_ROOM)),
        (from.max(middle + MARK_ROOM), to),
    ]
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
    /// What the line says of itself, in a gap at its middle, when this
    /// piece has the middle or half of it: how often it comes.
    mark: Option<(String, MarkAt)>,
}

/// Where the line runs down in its column, the dashes' length, about
/// how far from one to the next, how wide the bend to the dot is, and
/// how much of the line a mark takes, above and below its middle.
const CURL_X: f32 = 14.0;
const CURL_DASH: f32 = 2.5;
const CURL_STEP: f32 = 5.0;
const CURL_BEND: f32 = 4.0;
const MARK_ROOM: f32 = 8.0;

/// Dashes along a straight line from `from` to `to`, as many as fit,
/// each in the middle of its share: half a gap at either end.
fn dashes(frame: &mut Frame, stroke: Stroke<'_>, from: Point, to: Point) {
    let long = from.distance(to);
    if long < CURL_DASH {
        return;
    }
    let count = (long / CURL_STEP).round().max(1.0);
    let share = long / count;
    let along = |far: f32| {
        Point::new(
            from.x + (to.x - from.x) * far / long,
            from.y + (to.y - from.y) * far / long,
        )
    };
    for n in 0..count as usize {
        let start = share * n as f32 + (share - CURL_DASH) / 2.0;
        frame.stroke(&Path::line(along(start), along(start + CURL_DASH)), stroke);
    }
}

impl<Message> canvas::Program<Message> for CurlPiece {
    type State = Kept<(crate::look::Curl, Color, f32, Option<(String, MarkAt)>)>;

    fn draw(
        &self,
        kept: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        use crate::look::Curl;
        let key = (self.piece, self.color, self.dot, self.mark.clone());
        vec![kept.draw(key, renderer, bounds.size(), |frame| {
            let stroke = Stroke {
                line_cap: canvas::LineCap::Round,
                line_join: canvas::LineJoin::Round,
                ..Stroke::default().with_color(self.color).with_width(1.5)
            };
            let (tall, wide, dot) = (bounds.height, bounds.width, self.dot);
            // The run down this piece: all of it for one between, from
            // under the bend for the first, to over it for the last.
            let (from, to) = match self.piece {
                Curl::Past => (0.0, tall),
                Curl::Into => (dot + CURL_BEND, tall),
                Curl::Out => (0.0, dot - CURL_BEND),
            };
            let down = |frame: &mut Frame, from: f32, to: f32| {
                dashes(
                    frame,
                    stroke,
                    Point::new(CURL_X, from),
                    Point::new(CURL_X, to),
                );
            };
            match &self.mark {
                // What it says sits in a gap at the line's middle.
                Some((words, at)) => {
                    let middle = at.middle(dot, tall);
                    for (from, to) in round_mark(from, to, middle) {
                        down(frame, from, to);
                    }
                    frame.fill_text(canvas::Text {
                        content: words.clone(),
                        position: Point::new(CURL_X, middle),
                        color: self.color,
                        size: 9.5.into(),
                        font: crate::look::BOLD,
                        align_x: iced::widget::text::Alignment::Center,
                        align_y: iced::alignment::Vertical::Center,
                        ..canvas::Text::default()
                    });
                }
                None => down(frame, from, to),
            }
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
                Curl::Past => {}
                // In to the dot, with the arrow's head, from the line below.
                Curl::Into => {
                    frame.stroke(&bend(true), stroke);
                    dashes(
                        frame,
                        stroke,
                        Point::new(CURL_X + CURL_BEND + 1.0, dot),
                        Point::new(wide - 5.0, dot),
                    );
                    let head = Path::new(|path| {
                        path.move_to(Point::new(wide - 4.5, dot - 3.0));
                        path.line_to(Point::new(wide - 1.0, dot));
                        path.line_to(Point::new(wide - 4.5, dot + 3.0));
                    });
                    frame.stroke(&head, stroke);
                }
                // Down the line and out to the dot.
                Curl::Out => {
                    frame.stroke(&bend(false), stroke);
                    dashes(
                        frame,
                        stroke,
                        Point::new(CURL_X + CURL_BEND + 1.0, dot),
                        Point::new(wide, dot),
                    );
                }
            }
        })]
    }
}

/// The piece of the curl beside one step, the whole height it is given;
/// `dot` is how far down the step's dot is, and `mark` what the line
/// says of itself there, if this is the piece that says it.
pub fn curl<'a, Message: 'a>(
    piece: crate::look::Curl,
    color: Color,
    dot: f32,
    mark: Option<(String, MarkAt)>,
) -> iced::widget::Canvas<impl canvas::Program<Message> + 'a, Message> {
    draw(CurlPiece {
        piece,
        color,
        dot,
        mark,
    })
    .width(crate::look::CURL_WIDE)
    .height(Fill)
}

/// The line beside steps that come in a drawn order, the piece of it
/// beside one step: it wanders from side to side as it runs down, as a
/// loop's is dashed, and one row's piece meets the next.
struct WavePiece {
    /// Beside the first of them, one between, or the last.
    piece: crate::look::Curl,
    color: Color,
    dot: f32,
    /// The mark of a drawn order, in a gap at the line's middle, when
    /// this piece has the middle or half of it.
    mark: Option<MarkAt>,
}

/// About how tall each swing of the line is, and how far it swings.
const WAVE_TALL: f32 = 8.0;
const WAVE_OUT: f32 = 3.5;

impl<Message> canvas::Program<Message> for WavePiece {
    type State = Kept<(crate::look::Curl, Color, f32, Option<MarkAt>)>;

    fn draw(
        &self,
        kept: &Self::State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        use crate::look::Curl;
        let key = (self.piece, self.color, self.dot, self.mark);
        vec![kept.draw(key, renderer, bounds.size(), |frame| {
            let stroke = Stroke {
                line_cap: canvas::LineCap::Round,
                line_join: canvas::LineJoin::Round,
                ..Stroke::default().with_color(self.color).with_width(1.5)
            };
            let (tall, x) = (bounds.height, bounds.width / 2.0);
            let (from, to) = match self.piece {
                Curl::Past => (0.0, tall),
                Curl::Into => (self.dot, tall),
                Curl::Out => (0.0, self.dot),
            };
            // Swings down from `from` to `to`: an even number of them,
            // so a run ends on the side it began and meets the next.
            let wander = |frame: &mut Frame, from: f32, to: f32| {
                let long = to - from;
                if long < 2.0 {
                    return;
                }
                let swings = ((long / (2.0 * WAVE_TALL)).round().max(1.0) * 2.0) as usize;
                let each = long / swings as f32;
                let line = Path::new(|path| {
                    path.move_to(Point::new(x, from));
                    for n in 0..swings {
                        let side = if n % 2 == 0 { -WAVE_OUT } else { WAVE_OUT };
                        let top = from + each * n as f32;
                        path.quadratic_curve_to(
                            Point::new(x + side, top + each / 2.0),
                            Point::new(x, top + each),
                        );
                    }
                });
                frame.stroke(&line, stroke);
            };
            let Some(mark) = self.mark else {
                wander(frame, from, to);
                return;
            };
            // The mark of a drawn order: two lines that cross, each
            // with an arrow's head, small.
            let middle = mark.middle(self.dot, tall);
            for (from, to) in round_mark(from, to, middle) {
                wander(frame, from, to);
            }
            let at = |dx: f32, dy: f32| Point::new(x + dx * 0.72, middle + dy * 0.72);
            let lines: [&[(f32, f32)]; 5] = [
                &[(-7.0, -4.0), (-4.5, -4.0), (2.5, 4.0), (6.0, 4.0)],
                &[(-7.0, 4.0), (-4.5, 4.0), (-2.5, 1.7)],
                &[(1.5, -2.5), (2.5, -4.0), (6.0, -4.0)],
                &[(4.0, -6.5), (6.5, -4.0), (4.0, -1.5)],
                &[(4.0, 1.5), (6.5, 4.0), (4.0, 6.5)],
            ];
            for points in lines {
                let line = Path::new(|path| {
                    path.move_to(at(points[0].0, points[0].1));
                    for (dx, dy) in &points[1..] {
                        path.line_to(at(*dx, *dy));
                    }
                });
                frame.stroke(&line, stroke);
            }
        })]
    }
}

/// The piece of a drawn order's line beside one step: `piece` says
/// whether the step is the first of them, one between or the last, and
/// `mark` whether this piece carries the mark.
pub fn wave<'a, Message: 'a>(
    piece: crate::look::Curl,
    color: Color,
    dot: f32,
    mark: Option<MarkAt>,
) -> iced::widget::Canvas<impl canvas::Program<Message> + 'a, Message> {
    draw(WavePiece {
        piece,
        color,
        dot,
        mark,
    })
    .width(crate::look::DRAWN_WIDE)
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
