//! The line down the plan's steps, drawn once over the whole list: one
//! line through every step, whatever section it is in, with its dots,
//! what has played of the step that is on, and beside it the spans laid
//! over it: what comes again, to the left, and what comes in a drawn
//! order, to the right, each further out the more spans it holds.
//!
//! The rows say where they are ([`seen`]) and what should be drawn for
//! them ([`Picture`]); one canvas laid over the list ([`over`]) draws it
//! all from where the rows really are. So a line is as long as the way
//! between its two dots, a mark is at the middle of its line, and what
//! moves (a line filling, a ring opening, a line being drawn) moves along
//! the real thing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use iced::advanced::widget::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, overlay, renderer};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{
    Color, Element, Event, Fill, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse,
    window,
};
use lantern_engine::StepId;

use crate::drawn::Kept;

/// What says where it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A step's dot.
    Dot(StepId),
    /// The room beside a step for its dot and the lines either side.
    Row(StepId),
    /// The room right of a step for the brackets of the sections it is
    /// in.
    Side(StepId),
    /// The canvas itself: where what is laid over the list starts.
    Canvas,
    /// All of a row: a press outside every row is on nothing.
    Whole(StepId),
    /// What a fill proposes, with its buttons: not nothing either.
    Fill,
    /// Run's rail: the room for the dot of its `n`th row, and right of
    /// that row for the brackets.
    Road(usize),
    RoadSide(usize),
}

/// Where each of them is, as last laid out.
pub type Places = Rc<RefCell<HashMap<Key, Rectangle>>>;

/// `content`, which says where it is each time the window is drawn.
pub fn seen<'a, Message: 'a>(
    key: Key,
    places: &Places,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    Element::new(Seen {
        key,
        places: places.clone(),
        content: content.into(),
    })
}

struct Seen<'a, Message> {
    key: Key,
    places: Places,
    content: Element<'a, Message>,
}

impl<Message> Widget<Message, Theme, Renderer> for Seen<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // Before each drawing, so one out of sight says where it is too.
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let was = self.places.borrow_mut().insert(self.key, layout.bounds());
            // It moved: what is drawn over the list is worked out from
            // where things are, so it is asked for once more.
            if was != Some(layout.bounds()) {
                shell.request_redraw();
            }
        }
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.places.borrow_mut().insert(self.key, layout.bounds());
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

/// How a step's dot looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dot {
    /// Filled.
    Solid(Color),
    /// Its edge only.
    Hollow(Color),
    /// The step that is on: filled, in a ring, with a glow going out of
    /// it (`glow` of the way gone) when it has just gone up.
    On {
        color: Color,
        ring: Color,
        glow: Option<f32>,
    },
    /// One of the steps a pull would be about.
    Would(Color),
    /// Where a step that is being carried would land: a dashed ring.
    Place(Color),
    /// The steps of a folded section, gathered into one: three dots one
    /// over another, filled where they have played.
    Stack { color: Color, solid: bool },
}

impl Dot {
    /// How far from its middle it reaches.
    fn reach(self) -> f32 {
        match self {
            Dot::On { .. } => DOT + RING,
            Dot::Stack { .. } => DOT + 5.0,
            _ => DOT,
        }
    }

    fn color(self) -> Color {
        match self {
            Dot::Solid(color) | Dot::Hollow(color) | Dot::Would(color) | Dot::Place(color) => color,
            Dot::On { color, .. } | Dot::Stack { color, .. } => color,
        }
    }
}

/// The line from a step's dot to the next step's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Line {
    /// One colour, all of it.
    Plain(Color),
    /// The step is playing: `done` of the way along it in `played`, and
    /// from there on only its two sides, in `ahead`. `thick` of its usual
    /// width: less as the step fades out, and none once it has.
    Playing {
        done: f32,
        played: Color,
        ahead: Color,
        thick: f32,
    },
}

/// One step that is in sight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub id: StepId,
    pub dot: Dot,
    /// Its line down to the next step, where it goes on into it.
    pub line: Option<Line>,
    /// The step the line goes to, whatever section that is in: `None`
    /// for the last of the plan, whose line ends under it.
    pub next: Option<StepId>,
    /// The step is playing and the flow goes back from it when it ends:
    /// which of [`Picture::loops`] it follows. What has played is then
    /// shown along that line, not the one down.
    pub back: Option<(usize, f32, Color)>,
}

/// What a span says of itself, at the middle of its line.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    Nothing,
    /// How often it comes, or which round it is on.
    Words(String),
    /// The mark of a drawn order.
    Crossed,
}

/// A span laid over the line: from its first step in sight to its last.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub first: StepId,
    pub last: StepId,
    /// Whether the span really starts and ends at those two: not where
    /// its own first or last step is folded away.
    pub starts: bool,
    pub ends: bool,
    /// How many spans of its kind it holds: it is drawn that much
    /// further out from the dots.
    pub lane: u8,
    pub color: Color,
    pub mark: Mark,
    /// One a pull would make: it is not drawn in, it is there.
    pub pulled: bool,
    /// How far aside it is, as it shakes.
    pub shift: f32,
    /// The numbers of the notes on it.
    pub marks: Vec<usize>,
    /// The service just went round it: how far a light has run back up
    /// its line.
    pub pulse: Option<f32>,
}

/// Everything the canvas draws.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picture {
    /// The steps in sight, top to bottom.
    pub stops: Vec<Stop>,
    /// What comes again: dashed, curling back to where it starts.
    pub loops: Vec<Span>,
    /// What comes in a drawn order: a line that wanders.
    pub orders: Vec<Span>,
    /// Nothing moves: what would is where it is going at once.
    pub still: bool,
    /// The dots can be pressed: the one under the pointer shows it.
    pub hot: bool,
    /// Steps can be picked together by a box pulled from where there is
    /// nothing: its colour, where they can.
    pub select: Option<Color>,
    /// Every row in sight: a press on one is not a press
    /// on nothing.
    pub rows: Vec<StepId>,
    /// A fill is open: a press on what it proposes is not on nothing.
    pub filling: bool,
    /// The sections, each a bracket beside the steps it holds.
    pub brackets: Vec<Bracket>,
    /// The open note's threads: from its card to each thing it is on.
    pub threads: Vec<(Point, Point)>,
}

/// What a line beside a bracket is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// The time it is given.
    Goal,
    /// The time it takes, or has taken so far.
    Sum,
    /// A time of day.
    Clock,
    /// The time of day it starts.
    Start,
}

/// A section: a bracket to the right of the steps it holds, from the
/// first of them in sight to the last, with its name and its times
/// beside it. The plan is one too, round all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct Bracket {
    /// The section's heading; `None` for the plan's own, which has none.
    pub id: Option<StepId>,
    /// How far out it bulges, 0 to 1: it just took in what put it over
    /// its time.
    pub bulge: f32,
    pub first: StepId,
    pub last: StepId,
    /// How far right of where the brackets start its brace is: further
    /// out the more sections it holds, one in another.
    pub offset: f32,
    /// The steps its name and times are written beside, left of its
    /// brace, in the room the innermost brackets write theirs in: the
    /// first run of its steps that are in no section inside it. `None`
    /// writes them right of its brace.
    pub beside: Option<(StepId, StepId)>,
    pub color: Color,
    pub name: String,
    pub lines: Vec<(Sign, String, Color)>,
    /// Picked, or what something carried would go into.
    pub strong: bool,
    /// Its name and times are not written: something open over them
    /// says them.
    pub bare: bool,
    /// The numbers of the notes on it.
    pub marks: Vec<usize>,
    /// Where a section that is being carried would land: dashed, and
    /// what is written beside it faint.
    pub place: bool,
}

/// How much room a bracket takes, and what is written beside it.
pub const BRACE: f32 = 18.0;
pub const LABEL: f32 = 86.0;

/// Where a bracket's brace is: across, and from its top to its foot.
fn bracket_box(
    bracket: &Bracket,
    at: &dyn Fn(Key) -> Option<Rectangle>,
) -> Option<(f32, f32, f32)> {
    let first = at(Key::Side(bracket.first))?;
    let last = at(Key::Side(bracket.last))?;
    Some((
        first.x + bracket.offset + 3.0,
        first.y + 2.0,
        last.y + last.height - 4.0,
    ))
}

/// How much room right of the steps the brackets need, `lanes` deep:
/// the innermost and what is written beside it, and a brace for each
/// one further out; with room right of those too (`wide`) where one of
/// them has no steps of its own to write beside.
pub fn side_room(lanes: u8, wide: bool) -> f32 {
    match lanes {
        0 => 0.0,
        lanes => {
            BRACE
                + LABEL
                + BRACE * f32::from(lanes - 1)
                + match wide {
                    true => LABEL,
                    false => 0.0,
                }
        }
    }
}

/// How far right of where the brackets start the brace in `lane` is.
pub fn brace_at(lane: u8) -> f32 {
    match lane {
        0 => 0.0,
        lane => BRACE + LABEL + BRACE * f32::from(lane - 1),
    }
}

/// `name` on as few lines as it takes, two at most, each about as wide
/// as there is room for beside a bracket.
fn name_lines(name: &str) -> Vec<String> {
    const WIDE: usize = 13;
    let mut lines: Vec<String> = Vec::new();
    for word in name.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= WIDE => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    if lines.len() > 2 {
        let rest = lines.split_off(1).join(" ");
        lines.push(rest);
    }
    lines
        .into_iter()
        .map(|line| match line.chars().count() > WIDE {
            true => line.chars().take(WIDE - 1).chain(['…']).collect(),
            false => line,
        })
        .collect()
}

/// A dot's radius, and how much further out the ring of the one that is
/// on is.
const DOT: f32 = 5.0;
const RING: f32 = 4.0;
/// How wide a line is, each side of a line that has not played, and how
/// much room a line leaves what it passes.
const LINE: f32 = 2.0;
const SIDE: f32 = 0.5;
const ROOM: f32 = 2.0;
/// How far left of the dots the first loop's line runs, and how much
/// further each one that holds another: see [`loop_room`].
pub const LOOP_FIRST: f32 = 22.0;
pub const LOOP_STEP: f32 = 24.0;
/// The same for a drawn order's line, right of the dots.
pub const ORDER_FIRST: f32 = 17.0;
pub const ORDER_STEP: f32 = 12.0;
/// A loop's dashes' length, about how far from one to the next, how wide
/// its bend to a dot is, and how much of a line a mark takes, above and
/// below its middle.
const DASH: f32 = 2.5;
const DASH_STEP: f32 = 5.0;
const BEND: f32 = 4.0;
const MARK_ROOM: f32 = 8.0;
/// About how tall each swing of an order's line is, and how far it swings.
const WAVE_TALL: f32 = 8.0;
const WAVE_OUT: f32 = 3.5;
/// How long a line takes to be drawn, a ring to open, and how fast what
/// has played catches up with where it should be (seconds).
const DRAW: f32 = 0.26;
const OPEN: f32 = 0.3;
const CATCH_UP: f32 = 0.12;
/// How fast a bracket slides to where it should be (seconds).
const SLIDE: f32 = 0.07;

/// How much room left of the dots' middle the loops need, the deepest of
/// them `lanes` deep (0 for none), with what they say of themselves.
pub fn loop_room(lanes: u8) -> f32 {
    match lanes {
        0 => DOT + RING + 2.0,
        lanes => LOOP_FIRST + LOOP_STEP * f32::from(lanes - 1) + 16.0,
    }
}

/// How much room right of the dots' middle the drawn orders need.
pub fn order_room(lanes: u8) -> f32 {
    match lanes {
        0 => DOT + RING + 2.0,
        lanes => ORDER_FIRST + ORDER_STEP * f32::from(lanes - 1) + 7.0,
    }
}

/// Something that is drawn in when it first appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Thing {
    Line(StepId),
    Loop(StepId, StepId),
    Order(StepId, StepId),
    Bracket(Option<StepId>),
}

/// What is under way.
#[derive(Default)]
struct Moves {
    /// When it was last moved on.
    last: Option<Instant>,
    /// Whether anything has been drawn yet: what is there at first is
    /// there, not drawn in.
    begun: bool,
    /// How much of the playing step's line is shown as played.
    played: Option<(StepId, f32)>,
    /// How far open the ring of the step that is on is.
    ring: Option<(StepId, f32)>,
    /// How much of each is drawn.
    drawn: HashMap<Thing, f32>,
    /// The dot under the pointer.
    over: Option<StepId>,
    /// Where each bracket is shown, on its way to where it should be.
    boxes: HashMap<Option<StepId>, (f32, f32, f32)>,
}

/// How far the step that is playing has come, if one is.
fn playing(picture: &Picture) -> Option<(StepId, f32)> {
    picture
        .stops
        .iter()
        .find_map(|stop| match (stop.back, stop.line) {
            (Some((_, done, _)), _) => Some((stop.id, done)),
            (None, Some(Line::Playing { done, .. })) => Some((stop.id, done)),
            _ => None,
        })
}

impl Moves {
    /// Moves everything on by `passed` seconds towards what `picture`
    /// shows; whether anything is still on its way.
    fn on(
        &mut self,
        picture: &Picture,
        passed: f32,
        at: &dyn Fn(Key) -> Option<Rectangle>,
    ) -> bool {
        let fresh = !self.begun || picture.still;
        self.begun = true;
        let mut moving = false;
        // What has played.
        self.played = playing(picture).map(|(id, done)| {
            let from = match self.played {
                Some((was, shown)) if was == id && !fresh => shown,
                _ if fresh => done,
                _ => 0.0,
            };
            let near = 1.0 - (-passed / CATCH_UP).exp();
            let shown = from + (done - from) * near;
            match (done - shown).abs() < 0.004 {
                true => (id, done),
                false => {
                    moving = true;
                    (id, shown)
                }
            }
        });
        // The ring.
        let on = picture
            .stops
            .iter()
            .find(|stop| matches!(stop.dot, Dot::On { .. }))
            .map(|stop| stop.id);
        self.ring = on.map(|id| {
            let from = match self.ring {
                Some((was, open)) if was == id => open,
                _ if fresh => 1.0,
                _ => 0.0,
            };
            let open = (from + passed / OPEN).min(1.0);
            moving |= open < 1.0;
            (id, open)
        });
        // What is drawn in.
        let mut drawn = HashMap::new();
        let mut draw = |thing: Thing, there: bool| {
            let from = match self.drawn.get(&thing) {
                Some(far) => *far,
                None if fresh || there => 1.0,
                None => 0.0,
            };
            let far = match there || fresh {
                true => 1.0,
                false => (from + passed / DRAW).min(1.0),
            };
            moving |= far < 1.0;
            drawn.insert(thing, far);
        };
        for stop in &picture.stops {
            if stop.line.is_some() {
                draw(Thing::Line(stop.id), false);
            }
        }
        for span in &picture.loops {
            draw(Thing::Loop(span.first, span.last), span.pulled);
        }
        for span in &picture.orders {
            draw(Thing::Order(span.first, span.last), span.pulled);
        }
        for bracket in &picture.brackets {
            draw(Thing::Bracket(bracket.id), false);
        }
        self.drawn = drawn;
        // The brackets slide to where they should be.
        let near = 1.0 - (-passed / SLIDE).exp();
        let mut boxes = HashMap::new();
        for bracket in &picture.brackets {
            let Some(to) = bracket_box(bracket, at) else {
                continue;
            };
            let shown = match self.boxes.get(&bracket.id) {
                Some(was) if !fresh => {
                    let step = |a: f32, b: f32| match (b - a).abs() < 0.5 {
                        true => b,
                        false => a + (b - a) * near,
                    };
                    (step(was.0, to.0), step(was.1, to.1), step(was.2, to.2))
                }
                _ => to,
            };
            moving |= shown != to;
            boxes.insert(bracket.id, shown);
        }
        self.boxes = boxes;
        moving
    }

    fn drawn(&self, thing: Thing) -> f32 {
        ease(self.drawn.get(&thing).copied().unwrap_or(1.0))
    }

    /// How far the step `id` is shown to have played, `done` if it is
    /// not the one being caught up with.
    fn shown(&self, id: StepId, done: f32) -> f32 {
        match self.played {
            Some((was, shown)) if was == id => shown,
            _ => done,
        }
        .clamp(0.0, 1.0)
    }
}

/// Fast at first, slowing to a stop.
fn ease(far: f32) -> f32 {
    1.0 - (1.0 - far.clamp(0.0, 1.0)).powi(3)
}

/// What was drawn last, to be drawn again only when it changes.
type Shown = (
    Picture,
    HashMap<Key, Rectangle>,
    Option<(StepId, f32)>,
    Option<(StepId, f32)>,
    Vec<(Thing, f32)>,
    Option<StepId>,
    Option<Rectangle>,
    Vec<(Option<StepId>, i32, i32, i32)>,
);

#[derive(Default)]
struct State {
    /// The box being pulled from where there is nothing: where it began,
    /// where the pointer is, and whether it has moved far enough to be a
    /// box, not a click.
    band: Option<(Point, Point, bool)>,
    /// The steps the box was last over.
    banded: Vec<StepId>,
    kept: Kept<Shown>,
    moves: Moves,
}

struct Over<'a, Message> {
    picture: Picture,
    places: Places,
    /// What to say when a box has picked some steps, or a click on
    /// nothing has let go of them all.
    pick: Rc<dyn Fn(Vec<StepId>) -> Message + 'a>,
}

/// The box from one corner to the other.
fn boxed(from: Point, to: Point) -> Rectangle {
    Rectangle::new(
        Point::new(from.x.min(to.x), from.y.min(to.y)),
        Size::new((from.x - to.x).abs(), (from.y - to.y).abs()),
    )
}

impl<Message> canvas::Program<Message> for Over<'_, Message> {
    type State = State;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                let passed = state
                    .moves
                    .last
                    .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.1));
                let places = self.places.borrow().clone();
                let at = |key: Key| {
                    places.get(&key).map(|place| Rectangle {
                        x: place.x - bounds.x,
                        y: place.y - bounds.y,
                        ..*place
                    })
                };
                let moving = state.moves.on(&self.picture, passed, &at);
                state.moves.last = moving.then_some(*now);
                moving.then(canvas::Action::request_redraw)
            }
            // A press where there is no row begins a box; let go there,
            // it was a click on nothing, and nothing is picked.
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = cursor
                    .position()
                    .filter(|_| self.picture.select.is_some() && cursor.is_over(bounds))?;
                let places = self.places.borrow();
                let on_a_row = self.picture.rows.iter().any(|id| {
                    places
                        .get(&Key::Whole(*id))
                        .is_some_and(|row| row.contains(at))
                }) || (self.picture.filling
                    && places.get(&Key::Fill).is_some_and(|fill| fill.contains(at)));
                if !on_a_row {
                    state.band = Some((at, at, false));
                    state.banded.clear();
                }
                None
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let (_, _, moved) = state.band.take()?;
                Some(match moved {
                    true => canvas::Action::request_redraw(),
                    false => canvas::Action::publish((self.pick)(Vec::new())),
                })
            }
            // As the box is pulled, the steps it is over are picked.
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.band.is_some() => {
                let (from, _, moved) = state.band?;
                let at = cursor.position()?;
                let moved = moved || from.distance(at) > 4.0;
                state.band = Some((from, at, moved));
                if !moved {
                    return None;
                }
                let over = boxed(from, at);
                let places = self.places.borrow();
                let picked: Vec<StepId> = self
                    .picture
                    .stops
                    .iter()
                    .filter(|stop| {
                        places
                            .get(&Key::Whole(stop.id))
                            .is_some_and(|row| row.intersects(&over))
                    })
                    .map(|stop| stop.id)
                    .collect();
                Some(match picked == state.banded {
                    true => canvas::Action::request_redraw(),
                    false => {
                        state.banded = picked.clone();
                        canvas::Action::publish((self.pick)(picked))
                    }
                })
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let over = cursor
                    .position()
                    .filter(|_| self.picture.hot && cursor.is_over(bounds))
                    .and_then(|at| {
                        let places = self.places.borrow();
                        self.picture.stops.iter().find_map(|stop| {
                            places
                                .get(&Key::Dot(stop.id))
                                .filter(|dot| dot.center().distance(at) <= DOT + RING)
                                .map(|_| stop.id)
                        })
                    });
                (over != state.moves.over).then(|| {
                    state.moves.over = over;
                    canvas::Action::request_redraw()
                })
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        state: &State,
        renderer: &Renderer,
        _: &Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Where it starts, for what is laid over the list beside it.
        self.places.borrow_mut().insert(Key::Canvas, bounds);
        let places = self.places.borrow().clone();
        let moves = &state.moves;
        let mut drawn: Vec<(Thing, f32)> = moves
            .drawn
            .iter()
            .filter(|(_, far)| **far < 1.0)
            .map(|(thing, far)| (*thing, *far))
            .collect();
        drawn.sort_by(|a, b| a.1.total_cmp(&b.1));
        // The box being pulled, from the canvas's own corner.
        let band = state
            .band
            .filter(|(.., moved)| *moved)
            .map(|(from, to, _)| boxed(from, to))
            .map(|over| Rectangle {
                x: over.x - bounds.x,
                y: over.y - bounds.y,
                ..over
            });
        let shown = (
            self.picture.clone(),
            places.clone(),
            moves.played,
            moves.ring,
            drawn,
            moves.over,
            band,
            {
                let mut boxes: Vec<(Option<StepId>, i32, i32, i32)> = moves
                    .boxes
                    .iter()
                    .map(|(id, (x, top, foot))| (*id, *x as i32, *top as i32, *foot as i32))
                    .collect();
                boxes.sort_unstable();
                boxes
            },
        );
        vec![state.kept.draw(shown, renderer, bounds.size(), |frame| {
            // Where each is, from the canvas's own corner.
            let at = |key: Key| {
                places.get(&key).map(|place| Rectangle {
                    x: place.x - bounds.x,
                    y: place.y - bounds.y,
                    ..*place
                })
            };
            paint(frame, &self.picture, &at, moves);
            if let (Some(over), Some(ink)) = (band, self.picture.select) {
                frame.fill_rectangle(over.position(), over.size(), faded(ink, 0.14));
                frame.stroke(
                    &Path::rectangle(over.position(), over.size()),
                    Stroke::default()
                        .with_width(1.0)
                        .with_color(faded(ink, 0.7)),
                );
            }
        })]
    }
}

/// The canvas that draws `picture` over the list its rows are in: laid
/// over all of it, it takes no press and no pointer from what is under.
pub fn over<'a, Message: 'a>(
    picture: Picture,
    places: &Places,
    pick: impl Fn(Vec<StepId>) -> Message + 'a,
) -> Element<'a, Message> {
    iced::widget::canvas(Over {
        picture,
        places: places.clone(),
        pick: Rc::new(pick),
    })
    .width(Fill)
    .height(Fill)
    .into()
}

/// A straight part of a line, down or across.
type Run = (Point, Point);

fn long(run: &Run) -> f32 {
    run.0.distance(run.1)
}

/// The point `far` along `run`.
fn along(run: &Run, far: f32) -> Point {
    let whole = long(run).max(f32::EPSILON);
    Point::new(
        run.0.x + (run.1.x - run.0.x) * far / whole,
        run.0.y + (run.1.y - run.0.y) * far / whole,
    )
}

/// The box round the straight line from `from` to `to`, `wide` across.
fn bar(from: Point, to: Point, wide: f32) -> (Point, Size) {
    let (left, right) = (from.x.min(to.x), from.x.max(to.x));
    let (top, bottom) = (from.y.min(to.y), from.y.max(to.y));
    match right - left > bottom - top {
        // Across.
        true => (
            Point::new(left, top - wide / 2.0),
            Size::new(right - left, wide),
        ),
        // Down.
        false => (
            Point::new(left - wide / 2.0, top),
            Size::new(wide, bottom - top),
        ),
    }
}

/// `run` from `from` to `to` along it: all of the line, or its two sides.
fn part(frame: &mut Frame, run: &Run, from: f32, to: f32, color: Color, hollow: bool, thick: f32) {
    if to - from <= 0.0 || thick <= 0.0 {
        return;
    }
    let (line, side) = (LINE * thick, SIDE * thick);
    let (a, b) = (along(run, from), along(run, to));
    // A line that leans is drawn as one stroke.
    if (a.x - b.x).abs() > 0.01 && (a.y - b.y).abs() > 0.01 {
        frame.stroke(
            &Path::line(a, b),
            Stroke::default()
                .with_color(color)
                .with_width(match hollow {
                    true => side * 2.0,
                    false => line,
                }),
        );
        return;
    }
    if !hollow {
        let (corner, size) = bar(a, b, line);
        frame.fill_rectangle(corner, size, color);
        return;
    }
    let across = (run.1.x - run.0.x).abs() > (run.1.y - run.0.y).abs();
    for edge in [-1.0, 1.0] {
        let off = edge * (line - side) / 2.0;
        let shift = match across {
            true => Vector::new(0.0, off),
            false => Vector::new(off, 0.0),
        };
        let (corner, size) = bar(a + shift, b + shift, side);
        frame.fill_rectangle(corner, size, color);
    }
}

/// The way from the dot of the stop at `n` to the next step's, in
/// straight parts. To the foot of its own row where the step it goes to
/// is not in sight.
fn way(picture: &Picture, n: usize, at: &dyn Fn(Key) -> Option<Rectangle>) -> Vec<Run> {
    let stop = &picture.stops[n];
    let Some(dot) = at(Key::Dot(stop.id)) else {
        return Vec::new();
    };
    let x = dot.center_x();
    let y = dot.center_y() + stop.dot.reach();
    let mut runs: Vec<Run> = Vec::new();
    let down = |runs: &mut Vec<Run>, x: f32, from: f32, to: f32| {
        if to > from {
            runs.push((Point::new(x, from), Point::new(x, to)));
        }
    };
    let next = picture.stops[n + 1..]
        .iter()
        .find(|next| Some(next.id) == stop.next)
        .and_then(|next| Some((next, at(Key::Dot(next.id))?)));
    match next {
        Some((next, there)) => {
            let to = there.center_y() - next.dot.reach();
            let over = there.center_x();
            // Straight from the one to the other, like a thread: where
            // one of them is aside (as it shakes), the line leans.
            if to > y {
                runs.push((Point::new(x, y), Point::new(over, to)));
            }
        }
        None => {
            if let Some(row) = at(Key::Row(stop.id)) {
                let foot = row.y + row.height - ROOM;
                down(&mut runs, x, y, foot);
            }
        }
    }
    runs
}

fn paint(
    frame: &mut Frame,
    picture: &Picture,
    at: &dyn Fn(Key) -> Option<Rectangle>,
    moves: &Moves,
) {
    // The line from dot to dot.
    for (n, stop) in picture.stops.iter().enumerate() {
        let Some(line) = stop.line else {
            continue;
        };
        let runs = way(picture, n, at);
        let whole: f32 = runs.iter().map(long).sum();
        // As much of it as is drawn so far.
        let shown = whole * moves.drawn(Thing::Line(stop.id));
        let (played, ink, ahead, hollow, thick) = match line {
            Line::Plain(color) => (0.0, color, color, false, 1.0),
            // What has played is shown along the line it goes back by.
            Line::Playing { ahead, thick, .. } if stop.back.is_some() => {
                (0.0, ahead, ahead, false, thick)
            }
            Line::Playing {
                done,
                played,
                ahead,
                thick,
            } => (
                whole * moves.shown(stop.id, done),
                played,
                ahead,
                true,
                thick,
            ),
        };
        let mut before = 0.0;
        for run in &runs {
            let length = long(run);
            let end = (shown - before).min(length);
            let solid = (played - before).clamp(0.0, length).min(end.max(0.0));
            part(frame, run, 0.0, solid, ink, false, thick);
            part(frame, run, solid, end, ahead, hollow, thick);
            before += length;
        }
    }
    // The spans beside it.
    let going_back = picture
        .stops
        .iter()
        .find_map(|stop| Some((stop.id, stop.back?)));
    let pulling = picture.loops.iter().any(|span| span.pulled);
    for (n, span) in picture.loops.iter().enumerate() {
        let far = moves.drawn(Thing::Loop(span.first, span.last));
        // What is there steps back while another is pulled over it.
        let color = match pulling && !span.pulled {
            true => faded(span.color, 0.35),
            false => span.color,
        };
        let played = going_back
            .filter(|(_, (back, ..))| *back == n)
            .map(|(id, (_, done, color))| (moves.shown(id, done), color));
        curl(frame, span, color, far, played, at);
    }
    let pulling = picture.orders.iter().any(|span| span.pulled);
    for span in &picture.orders {
        let far = moves.drawn(Thing::Order(span.first, span.last));
        let color = match pulling && !span.pulled {
            true => faded(span.color, 0.35),
            false => span.color,
        };
        wander(frame, span, color, far, at);
    }
    // The sections' brackets.
    for bracket in &picture.brackets {
        let (Some(first), Some(last)) = (at(Key::Side(bracket.first)), at(Key::Side(bracket.last)))
        else {
            continue;
        };
        let far = moves.drawn(Thing::Bracket(bracket.id));
        // Where it is on its way to where it should be: it slides out a
        // lane, and stretches to take in what comes into it.
        let Some((x, top, foot)) = moves
            .boxes
            .get(&bracket.id)
            .copied()
            .or_else(|| bracket_box(bracket, at))
        else {
            continue;
        };
        let _ = (first, last);
        if foot - top < 12.0 {
            continue;
        }
        let middle = (top + foot) / 2.0;
        // It opens out from its middle as it is made.
        let (top, foot) = (
            middle - (middle - top) * far,
            middle + (foot - middle) * far,
        );
        let color = faded(
            bracket.color,
            match bracket.place {
                true => far * 0.6,
                false => far,
            },
        );
        let round = 7.0_f32.min((foot - top) / 4.0);
        let brace = Path::new(|path| {
            path.move_to(Point::new(x, top));
            path.quadratic_curve_to(
                Point::new(x + round, top),
                Point::new(x + round, top + round),
            );
            path.line_to(Point::new(x + round, middle - round));
            path.quadratic_curve_to(
                Point::new(x + round + 10.0 * bracket.bulge, middle),
                Point::new(x + round + 6.0 + 10.0 * bracket.bulge, middle),
            );
            path.quadratic_curve_to(
                Point::new(x + round + 10.0 * bracket.bulge, middle),
                Point::new(x + round, middle + round),
            );
            path.line_to(Point::new(x + round, foot - round));
            path.quadratic_curve_to(Point::new(x + round, foot), Point::new(x, foot));
        });
        frame.stroke(
            &brace,
            Stroke {
                line_cap: canvas::LineCap::Round,
                line_join: canvas::LineJoin::Round,
                line_dash: canvas::LineDash {
                    segments: match bracket.place {
                        true => &[4.0, 4.0],
                        false => &[],
                    },
                    offset: 0,
                },
                ..Stroke::default()
                    .with_color(color)
                    .with_width(match bracket.strong {
                        true => 2.6,
                        false => 1.6,
                    })
            },
        );
        if bracket.bare {
            continue;
        }
        // Beside it: its name, and as many of its times as there is
        // room for.
        const TALL: f32 = 14.0;
        let names = name_lines(&bracket.name);
        // Right of its brace; or left of it, beside the steps that are
        // its own, where the innermost write theirs.
        let own = bracket
            .beside
            .and_then(|(from, to)| at(Key::Side(from)).zip(at(Key::Side(to))));
        let (left, middle, tall) = match own {
            Some((from, to)) => (
                first.x + 3.0 + 7.0 + 12.0,
                (from.y + to.y + to.height) / 2.0,
                to.y + to.height - from.y,
            ),
            None => (x + round + 12.0, middle, foot - top),
        };
        let room = ((tall + 10.0) / TALL).floor().max(1.0) as usize;
        let names = &names[..names.len().min(room)];
        let lines = &bracket.lines[..bracket.lines.len().min(room - names.len())];
        let tall = TALL * (names.len() + lines.len()) as f32;
        let mut y = middle - tall / 2.0 + TALL / 2.0;
        for name in names {
            frame.fill_text(canvas::Text {
                content: name.clone(),
                position: Point::new(left, y),
                color,
                size: 11.5.into(),
                font: crate::look::BOLD,
                align_y: iced::alignment::Vertical::Center,
                ..canvas::Text::default()
            });
            y += TALL;
        }
        for (sign, words, ink) in lines {
            let ink = faded(*ink, far);
            let stroke = Stroke {
                line_cap: canvas::LineCap::Round,
                line_join: canvas::LineJoin::Round,
                ..Stroke::default().with_color(ink).with_width(1.2)
            };
            let c = Point::new(left + 4.5, y);
            match sign {
                Sign::Goal => {
                    frame.stroke(&Path::circle(c, 4.0), stroke);
                    frame.fill(&Path::circle(c, 1.3), ink);
                }
                Sign::Clock => {
                    frame.stroke(&Path::circle(c, 4.0), stroke);
                    frame.stroke(
                        &Path::new(|path| {
                            path.move_to(Point::new(c.x, c.y - 2.4));
                            path.line_to(c);
                            path.line_to(Point::new(c.x + 2.0, c.y));
                        }),
                        stroke,
                    );
                }
                Sign::Sum => frame.stroke(
                    &Path::new(|path| {
                        path.move_to(Point::new(c.x + 3.5, c.y - 4.0));
                        path.line_to(Point::new(c.x - 3.5, c.y - 4.0));
                        path.line_to(Point::new(c.x + 0.5, c.y));
                        path.line_to(Point::new(c.x - 3.5, c.y + 4.0));
                        path.line_to(Point::new(c.x + 3.5, c.y + 4.0));
                    }),
                    stroke,
                ),
                Sign::Start => frame.fill(
                    &Path::new(|path| {
                        path.move_to(Point::new(c.x - 3.0, c.y - 4.0));
                        path.line_to(Point::new(c.x + 4.0, c.y));
                        path.line_to(Point::new(c.x - 3.0, c.y + 4.0));
                        path.close();
                    }),
                    ink,
                ),
            }
            frame.fill_text(canvas::Text {
                content: words.clone(),
                position: Point::new(left + 13.0, y),
                color: ink,
                size: 10.5.into(),
                align_y: iced::alignment::Vertical::Center,
                ..canvas::Text::default()
            });
            y += TALL;
        }
        // Its notes' marks, under what is written.
        for (n, number) in bracket.marks.iter().enumerate() {
            note_mark(frame, Point::new(left + 6.5 + 16.0 * n as f32, y), *number);
        }
    }
    // The open note's threads, from its card to what it is on.
    for (from, to) in &picture.threads {
        let thread = Path::new(|path| {
            path.move_to(*from);
            path.bezier_curve_to(
                Point::new(from.x, from.y - 20.0),
                Point::new(to.x, to.y + 20.0),
                *to,
            );
        });
        frame.stroke(
            &thread,
            Stroke {
                line_dash: canvas::LineDash {
                    segments: &[2.0, 3.0],
                    offset: 0,
                },
                ..Stroke::default()
                    .with_width(1.2)
                    .with_color(Color::from_rgba(0.953, 0.835, 0.541, 0.8))
            },
        );
    }
    // The dots, over the ends of the lines.
    for stop in &picture.stops {
        let Some(place) = at(Key::Dot(stop.id)) else {
            continue;
        };
        let middle = place.center();
        if moves.over == Some(stop.id) {
            frame.fill(
                &Path::circle(middle, DOT + RING),
                faded(stop.dot.color(), 0.22),
            );
        }
        dot(frame, stop, middle, moves);
    }
}

fn dot(frame: &mut Frame, stop: &Stop, middle: Point, moves: &Moves) {
    let glow = |frame: &mut Frame, color: Color, gone: f32| {
        frame.stroke(
            &Path::circle(middle, DOT + 1.5 + 9.0 * gone),
            Stroke::default()
                .with_width(2.5)
                .with_color(faded(color, 0.6 * (1.0 - gone))),
        );
    };
    match stop.dot {
        Dot::Solid(color) => frame.fill(&Path::circle(middle, DOT), color),
        Dot::Hollow(color) => frame.stroke(
            &Path::circle(middle, DOT - 1.0),
            Stroke::default().with_width(2.0).with_color(color),
        ),
        Dot::Stack { color, solid } => {
            for (aside, fade) in [(-5.0, 0.35), (0.0, 0.6), (5.0, 1.0)] {
                let at = Point::new(middle.x + aside, middle.y);
                let ink = Color {
                    a: color.a * fade,
                    ..color
                };
                if solid {
                    frame.fill(&Path::circle(at, DOT - 0.5), ink);
                }
                frame.stroke(
                    &Path::circle(at, DOT - 1.0),
                    Stroke::default().with_width(1.6).with_color(ink),
                );
            }
        }
        Dot::Place(color) => frame.stroke(
            &Path::circle(middle, DOT - 1.0),
            Stroke {
                line_dash: canvas::LineDash {
                    segments: &[2.0, 2.0],
                    offset: 0,
                },
                ..Stroke::default().with_width(2.0).with_color(color)
            },
        ),
        Dot::Would(color) => {
            glow(frame, color, 0.3);
            frame.fill(&Path::circle(middle, DOT), color);
        }
        Dot::On {
            color,
            ring,
            glow: gone,
        } => {
            // The ring opens out from the dot as the step goes on.
            let open = match moves.ring {
                Some((id, open)) if id == stop.id => ease(open),
                _ => 1.0,
            };
            if let Some(gone) = gone {
                glow(frame, color, gone);
            }
            frame.stroke(
                &Path::circle(middle, DOT + (RING - 0.75) * open),
                Stroke::default()
                    .with_width(1.5)
                    .with_color(faded(ring, open)),
            );
            frame.fill(&Path::circle(middle, DOT), color);
        }
    }
}

/// A note's mark, numbered `n`, its middle at `at`.
fn note_mark(frame: &mut Frame, at: Point, n: usize) {
    const INK: Color = Color::from_rgb(0.953, 0.835, 0.541);
    frame.fill(
        &Path::rounded_rectangle(
            Point::new(at.x - 6.5, at.y - 6.5),
            Size::new(13.0, 13.0),
            3.0.into(),
        ),
        INK,
    );
    frame.fill_text(canvas::Text {
        content: n.to_string(),
        position: at,
        color: Color::from_rgb(0.09, 0.09, 0.11),
        size: 9.0.into(),
        font: crate::look::BOLD,
        align_x: iced::widget::text::Alignment::Center,
        align_y: iced::alignment::Vertical::Center,
        ..canvas::Text::default()
    });
}

/// `color`, `strength` as strong.
fn faded(color: Color, strength: f32) -> Color {
    Color {
        a: color.a * strength,
        ..color
    }
}

/// The straight line from `from` to `to`: solid for the first `solid`
/// of its length, then dashes, as many as fit the whole of it, each in
/// the middle of its share. Nothing past `upto` of its length is drawn.
fn dashes(
    frame: &mut Frame,
    stroke: Stroke<'_>,
    from: Point,
    to: Point,
    upto: f32,
    solid: Option<(f32, Color)>,
) {
    let run = (from, to);
    let whole = long(&run);
    let filled = solid.map_or(0.0, |(far, _)| far.clamp(0.0, whole).min(upto.max(0.0)));
    if let Some((_, color)) = solid
        && filled > 0.0
    {
        frame.stroke(
            &Path::line(from, along(&run, filled)),
            Stroke {
                line_cap: canvas::LineCap::Round,
                ..Stroke::default().with_color(color).with_width(LINE)
            },
        );
    }
    if whole < DASH {
        return;
    }
    let count = (whole / DASH_STEP).round().max(1.0);
    let share = whole / count;
    for n in 0..count as usize {
        let start = share * n as f32 + (share - DASH) / 2.0;
        if start + DASH > upto {
            return;
        }
        if start < filled {
            continue;
        }
        frame.stroke(
            &Path::line(along(&run, start), along(&run, start + DASH)),
            stroke,
        );
    }
}

/// The parts of the run from `from` up to `to` (which is above it) that
/// a mark with its middle at `middle` leaves.
fn round_mark(from: f32, to: f32, middle: Option<f32>) -> Vec<(f32, f32)> {
    match middle {
        Some(middle) => vec![
            (from, to.max(middle + MARK_ROOM)),
            (from.min(middle - MARK_ROOM), to),
        ],
        None => vec![(from, to)],
    }
}

/// Where a span's two ends are: the middle of its first and last steps'
/// dots; and the rows of those two, for a span that runs on out of sight.
struct Ends {
    first: Rectangle,
    last: Rectangle,
    first_row: Option<Rectangle>,
    last_row: Option<Rectangle>,
}

fn ends(span: &Span, at: &dyn Fn(Key) -> Option<Rectangle>) -> Option<Ends> {
    Some(Ends {
        first: at(Key::Dot(span.first))?,
        last: at(Key::Dot(span.last))?,
        first_row: at(Key::Row(span.first)),
        last_row: at(Key::Row(span.last)),
    })
}

/// The line of what comes again: out from its last step's dot, up beside
/// the steps, and in to where it starts with an arrow's head: its first
/// step's dot. `far` of it is drawn, from where it leaves the last step; and
/// where the step that is playing goes back by it, `played` of it is
/// solid in that colour.
fn curl(
    frame: &mut Frame,
    span: &Span,
    color: Color,
    far: f32,
    played: Option<(f32, Color)>,
    at: &dyn Fn(Key) -> Option<Rectangle>,
) {
    let Some(ends) = ends(span, at) else {
        return;
    };
    let stroke = Stroke {
        line_cap: canvas::LineCap::Round,
        line_join: canvas::LineJoin::Round,
        ..Stroke::default().with_color(color).with_width(1.5)
    };
    let rail = ends.last.center_x();
    let x = rail - LOOP_FIRST - LOOP_STEP * f32::from(span.lane) + span.shift;
    let (top, bottom) = (ends.first.center_y(), ends.last.center_y());
    // Where it turns, or runs on out of sight where its own end is
    // folded away.
    let from = match (span.ends, ends.last_row) {
        (false, Some(row)) => row.y + row.height,
        _ => bottom - BEND,
    };
    let to = match (span.starts, ends.first_row) {
        (false, Some(row)) => row.y,
        _ => top + BEND,
    };
    let tip = ends.first.center_x() - DOT - RING - 2.0;
    let out = (rail - DOT - RING - 1.0 - (x + BEND + 1.0)).max(0.0);
    let into = (tip - 4.0 - (x + BEND + 1.0)).max(0.0);
    let up = (from - to).max(0.0);
    let whole = out + up + into;
    // Gone round: a light runs back along it, out, up and in.
    if let Some(t) = span.pulse {
        let run = whole * t;
        let at = if run < out {
            Point::new(x + BEND + 1.0 + out - run, bottom)
        } else if run < out + up {
            Point::new(x, from - (run - out))
        } else {
            Point::new(x + BEND + 1.0 + (run - out - up).min(into), top)
        };
        let fade = 1.0 - t * t;
        let light = crate::motion::blend(color, Color::WHITE, 0.7);
        frame.fill(&Path::circle(at, 9.0), faded(light, 0.18 * fade));
        frame.fill(&Path::circle(at, 5.5), faded(light, 0.35 * fade));
        frame.fill(&Path::circle(at, 3.5), faded(light, fade));
    }
    // The words say the new round brightly while the light runs.
    let color = match span.pulse {
        Some(t) => crate::motion::blend(Color::WHITE, color, t),
        None => color,
    };
    let mut left = whole * far;
    // How much of it has played, from where it leaves.
    let filled =
        std::cell::Cell::new(played.map(|(done, color)| (whole * done.clamp(0.0, 1.0), color)));
    let take = |length: f32| {
        let now = filled.get();
        if let Some((far, color)) = now {
            filled.set(Some(((far - length).max(0.0), color)));
        }
        now.filter(|(far, _)| *far > 0.0)
    };
    let bend = |frame: &mut Frame, path: &Path, solid: Option<(f32, Color)>| match solid {
        Some((_, color)) => frame.stroke(
            path,
            Stroke {
                line_cap: canvas::LineCap::Round,
                ..Stroke::default().with_color(color).with_width(LINE)
            },
        ),
        None => frame.stroke(path, stroke),
    };
    // Out from the last step's dot.
    if span.ends {
        dashes(
            frame,
            stroke,
            Point::new(x + BEND + 1.0 + out, bottom),
            Point::new(x + BEND + 1.0, bottom),
            left,
            take(out),
        );
        if left >= out {
            let turn = Path::new(|path| {
                path.move_to(Point::new(x + BEND, bottom));
                path.quadratic_curve_to(Point::new(x, bottom), Point::new(x, bottom - BEND));
            });
            bend(frame, &turn, filled.get().filter(|(far, _)| *far > 0.0));
        }
        left -= out;
    }
    // Up, round what it says of itself at its middle.
    let words = match &span.mark {
        Mark::Words(words) if up > MARK_ROOM * 3.0 => Some(words),
        _ => None,
    };
    let middle = (from + to) / 2.0;
    let mut gone = 0.0;
    for (from, to) in round_mark(from, to, words.map(|_| middle)) {
        if from > to {
            dashes(
                frame,
                stroke,
                Point::new(x, from),
                Point::new(x, to),
                left - gone,
                take(from - to),
            );
            gone += from - to;
        }
    }
    // Its notes' marks, beside its middle, out from the steps.
    if left >= up / 2.0 {
        for (n, number) in span.marks.iter().enumerate() {
            note_mark(
                frame,
                Point::new(x - 13.0 - 15.0 * n as f32, middle),
                *number,
            );
        }
    }
    if let Some(words) = words
        && left >= up / 2.0
    {
        frame.fill_text(canvas::Text {
            content: words.clone(),
            position: Point::new(x, middle),
            color,
            size: 9.5.into(),
            font: crate::look::BOLD,
            align_x: iced::widget::text::Alignment::Center,
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
    }
    left -= up;
    // In to where it starts, with the arrow's head.
    if span.starts && left > 0.0 {
        let turn = Path::new(|path| {
            path.move_to(Point::new(x, top + BEND));
            path.quadratic_curve_to(Point::new(x, top), Point::new(x + BEND, top));
        });
        bend(frame, &turn, filled.get().filter(|(far, _)| *far > 0.0));
        dashes(
            frame,
            stroke,
            Point::new(x + BEND + 1.0, top),
            Point::new(x + BEND + 1.0 + into, top),
            left,
            take(into),
        );
        if left >= into {
            frame.stroke(
                &Path::new(|path| {
                    path.move_to(Point::new(tip - 3.5, top - 3.0));
                    path.line_to(Point::new(tip, top));
                    path.line_to(Point::new(tip - 3.5, top + 3.0));
                }),
                stroke,
            );
        }
    }
}

/// The line of a drawn order: it wanders from side to side as it runs
/// down beside the steps, with its mark in a gap at its middle. `far` of
/// it is drawn, from the top.
fn wander(
    frame: &mut Frame,
    span: &Span,
    color: Color,
    far: f32,
    at: &dyn Fn(Key) -> Option<Rectangle>,
) {
    let Some(ends) = ends(span, at) else {
        return;
    };
    let stroke = Stroke {
        line_cap: canvas::LineCap::Round,
        line_join: canvas::LineJoin::Round,
        ..Stroke::default().with_color(color).with_width(1.5)
    };
    let x = ends.last.center_x() + ORDER_FIRST + ORDER_STEP * f32::from(span.lane) + span.shift;
    let top = match (span.starts, ends.first_row) {
        (false, Some(row)) => row.y,
        _ => ends.first.center_y(),
    };
    let bottom = match (span.ends, ends.last_row) {
        (false, Some(row)) => row.y + row.height,
        _ => ends.last.center_y(),
    };
    let whole = bottom - top;
    if whole < 2.0 {
        return;
    }
    let shown = top + whole * far;
    // Swings down from `from` to `to`: an even number of them, so a run
    // ends on the side it began. Only those above `shown` are drawn.
    let swing = |frame: &mut Frame, from: f32, to: f32| {
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
                if top + each > shown + 0.01 {
                    break;
                }
                path.quadratic_curve_to(
                    Point::new(x + side, top + each / 2.0),
                    Point::new(x, top + each),
                );
            }
        });
        frame.stroke(&line, stroke);
    };
    let middle = (top + bottom) / 2.0;
    // Its notes' marks, beside its middle, out from the steps.
    if shown >= middle {
        for (n, number) in span.marks.iter().enumerate() {
            note_mark(
                frame,
                Point::new(x + 13.0 + 15.0 * n as f32, middle),
                *number,
            );
        }
    }
    if span.mark != Mark::Crossed || whole < MARK_ROOM * 3.0 {
        swing(frame, top, bottom);
        return;
    }
    swing(frame, top, middle - MARK_ROOM);
    swing(frame, middle + MARK_ROOM, bottom);
    if shown < middle {
        return;
    }
    // The mark of a drawn order: two lines that cross, each with an
    // arrow's head, small.
    let point = |dx: f32, dy: f32| Point::new(x + dx * 0.72, middle + dy * 0.72);
    let lines: [&[(f32, f32)]; 5] = [
        &[(-7.0, -4.0), (-4.5, -4.0), (2.5, 4.0), (6.0, 4.0)],
        &[(-7.0, 4.0), (-4.5, 4.0), (-2.5, 1.7)],
        &[(1.5, -2.5), (2.5, -4.0), (6.0, -4.0)],
        &[(4.0, -6.5), (6.5, -4.0), (4.0, -1.5)],
        &[(4.0, 1.5), (6.5, 4.0), (4.0, 6.5)],
    ];
    for points in lines {
        let line = Path::new(|path| {
            path.move_to(point(points[0].0, points[0].1));
            for (dx, dy) in &points[1..] {
                path.line_to(point(*dx, *dy));
            }
        });
        frame.stroke(&line, stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> StepId {
        serde_json::from_str(&n.to_string()).unwrap()
    }

    fn stop(n: u64, line: Option<Line>, next: Option<u64>) -> Stop {
        Stop {
            id: id(n),
            dot: Dot::Solid(Color::WHITE),
            line,
            next: next.map(id),
            back: None,
        }
    }

    /// Dots 10 across, their middles at x 25 and as far down as given.
    fn places<const N: usize>(of: [(Key, f32); N]) -> impl Fn(Key) -> Option<Rectangle> {
        let of: HashMap<Key, f32> = of.into_iter().collect();
        move |key| {
            of.get(&key).map(|y| match key {
                Key::Row(_) => Rectangle::new(Point::new(0.0, y - 20.0), Size::new(50.0, 40.0)),
                _ => Rectangle::new(Point::new(20.0, y - 5.0), Size::new(10.0, 10.0)),
            })
        }
    }

    #[test]
    fn a_line_runs_from_one_dot_to_the_next() {
        let picture = Picture {
            stops: vec![
                stop(1, Some(Line::Plain(Color::WHITE)), Some(2)),
                stop(2, None, None),
            ],
            ..Picture::default()
        };
        let at = places([(Key::Dot(id(1)), 20.0), (Key::Dot(id(2)), 80.0)]);
        let runs = way(&picture, 0, &at);
        assert_eq!(runs, vec![(Point::new(25.0, 25.0), Point::new(25.0, 75.0))]);
    }

    #[test]
    fn a_line_to_a_step_out_of_sight_ends_at_the_foot_of_its_row() {
        let picture = Picture {
            stops: vec![
                stop(1, Some(Line::Plain(Color::WHITE)), None),
                stop(2, Some(Line::Plain(Color::WHITE)), Some(9)),
            ],
            ..Picture::default()
        };
        let at = places([
            (Key::Dot(id(1)), 20.0),
            (Key::Row(id(1)), 20.0),
            (Key::Dot(id(2)), 60.0),
            (Key::Row(id(2)), 60.0),
        ]);
        // The last of the plan.
        assert_eq!(
            way(&picture, 0, &at),
            vec![(Point::new(25.0, 25.0), Point::new(25.0, 38.0))]
        );
        // One whose next step is folded away.
        assert_eq!(
            way(&picture, 1, &at),
            vec![(Point::new(25.0, 65.0), Point::new(25.0, 78.0))]
        );
    }

    #[test]
    fn each_span_that_holds_another_takes_more_room() {
        assert!(loop_room(0) < loop_room(1));
        assert_eq!(loop_room(2) - loop_room(1), LOOP_STEP);
        assert_eq!(order_room(2) - order_room(1), ORDER_STEP);
    }

    #[test]
    fn what_has_played_catches_up_and_a_new_line_is_drawn_in() {
        let playing = |done| Picture {
            stops: vec![stop(
                1,
                Some(Line::Playing {
                    done,
                    played: Color::WHITE,
                    ahead: Color::BLACK,
                    thick: 1.0,
                }),
                None,
            )],
            ..Picture::default()
        };
        let mut moves = Moves::default();
        // What is there at first is there.
        assert!(!moves.on(&playing(0.25), 0.0, &|_| None));
        assert_eq!(moves.played, Some((id(1), 0.25)));
        assert_eq!(moves.drawn(Thing::Line(id(1))), 1.0);
        // A jump is caught up with, not made at once.
        assert!(moves.on(&playing(0.75), 0.05, &|_| None));
        let shown = moves.shown(id(1), 0.75);
        assert!(shown > 0.25 && shown < 0.75);
        for _ in 0..100 {
            moves.on(&playing(0.75), 0.05, &|_| None);
        }
        assert_eq!(moves.played, Some((id(1), 0.75)));
        // A line that was not there is drawn in.
        let mut two = playing(0.75);
        two.stops
            .push(stop(2, Some(Line::Plain(Color::WHITE)), None));
        assert!(moves.on(&two, 0.05, &|_| None));
        assert!(moves.drawn(Thing::Line(id(2))) < 1.0);
        // With motion off it is there at once.
        two.still = true;
        two.stops
            .push(stop(3, Some(Line::Plain(Color::WHITE)), None));
        assert!(!moves.on(&two, 0.05, &|_| None));
        assert_eq!(moves.drawn(Thing::Line(id(3))), 1.0);
    }

    #[test]
    fn a_step_that_goes_back_is_caught_up_with_along_that_line() {
        let mut picture = Picture {
            stops: vec![stop(1, None, None), stop(2, None, None)],
            ..Picture::default()
        };
        picture.stops[1].back = Some((0, 0.5, Color::WHITE));
        let mut moves = Moves::default();
        moves.on(&picture, 0.0, &|_| None);
        assert_eq!(moves.played, Some((id(2), 0.5)));
        assert_eq!(moves.shown(id(2), 0.5), 0.5);
    }

    #[test]
    fn a_name_beside_a_bracket_takes_two_lines_at_most() {
        assert_eq!(name_lines("Culto"), vec!["Culto"]);
        assert_eq!(
            name_lines("Servicio de Canto"),
            vec!["Servicio de", "Canto"]
        );
        assert_eq!(
            name_lines("Escuela Sabática de los menores"),
            vec!["Escuela", "Sabática de …"]
        );
    }
}
