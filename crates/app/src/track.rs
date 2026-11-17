//! The line down the plan's steps, drawn once over the whole list: the
//! dots, the lines from one to the next, what has played of the one that
//! is on, and beside them the line of what comes again and of a drawn
//! order.
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
    /// A section's dot.
    Section(StepId),
    /// Rows cut to a height while their section closes or opens.
    Cut(u32),
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
            self.places.borrow_mut().insert(self.key, layout.bounds());
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
}

impl Dot {
    /// How far from its middle it reaches.
    fn reach(self) -> f32 {
        match self {
            Dot::On { .. } => DOT + RING,
            _ => DOT,
        }
    }

    fn color(self) -> Color {
        match self {
            Dot::Solid(color) | Dot::Hollow(color) | Dot::Would(color) => color,
            Dot::On { color, .. } => color,
        }
    }
}

/// The line from a step's dot to the next step's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Line {
    /// One colour, all of it.
    Plain(Color),
    /// The step is playing: `done` of the way along it in `played`, and
    /// from there on only its two sides, in `ahead`.
    Playing {
        done: f32,
        played: Color,
        ahead: Color,
    },
}

/// One step that is in sight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub id: StepId,
    pub dot: Dot,
    pub line: Option<Line>,
    /// The step the line goes to: `None` for the last of the plan, whose
    /// line ends under it.
    pub next: Option<StepId>,
    /// The section it is in, and whether that one is being worked on:
    /// what is drawn for the steps of one that is not steps back.
    pub group: Option<StepId>,
    pub awake: bool,
    /// The cut it is under, while its section closes or opens.
    pub cut: Option<u32>,
}

/// What is down the rail, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Item {
    Stop(Stop),
    /// A section's heading: a line that passes it leaves its dot room.
    Section {
        id: StepId,
        cut: Option<u32>,
    },
}

/// What a line beside the steps says of itself, at its middle.
#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    Nothing,
    /// How often it comes.
    Words(String),
    /// The mark of a drawn order.
    Crossed,
}

/// A line beside some steps: the first and last of them in sight.
#[derive(Debug, Clone, PartialEq)]
pub struct Beside {
    pub first: StepId,
    pub last: StepId,
    /// Whether the line really starts and ends at those two: not where
    /// its own first or last step is folded away.
    pub starts: bool,
    pub ends: bool,
    pub color: Color,
    pub mark: Mark,
    /// One a pull would make: it is not drawn in, it is there.
    pub pulled: bool,
    /// The section its steps are in.
    pub group: Option<StepId>,
}

/// A section's own line, down from its dot: on to the next section's
/// dot where it hands over by itself, and curling back from the level of
/// its last step where it comes again.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub id: StepId,
    /// The last of its steps in sight.
    pub last: Option<StepId>,
    /// The section it hands over to, where it does and that one is next
    /// down the same line.
    pub next: Option<StepId>,
    /// What its loop says of itself, where it comes again.
    pub again: Option<Mark>,
    pub color: Color,
    pub cut: Option<u32>,
}

/// Everything the canvas draws.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Picture {
    pub items: Vec<Item>,
    /// The sections' own lines.
    pub sections: Vec<Section>,
    /// What comes again: dashed, curling back to its first step.
    pub loops: Vec<Beside>,
    /// What comes in a drawn order: a line that wanders.
    pub orders: Vec<Beside>,
    /// How wide the room for a loop's line is, left of the dots, and for
    /// an order's, right of them.
    pub curl_wide: f32,
    pub order_wide: f32,
    /// Nothing moves: what would is where it is going at once.
    pub still: bool,
    /// The dots can be pressed: the one under the pointer shows it.
    pub hot: bool,
}

/// A dot's radius, and how much further out the ring of the one that is
/// on is.
const DOT: f32 = 5.0;
const RING: f32 = 4.0;
/// How wide a line is, each side of a line that has not played, and how
/// much room a line leaves a section's dot.
const LINE: f32 = 2.0;
const SIDE: f32 = 0.5;
const ROOM: f32 = 2.0;
/// Where a loop's line runs down in its room, its dashes' length, about
/// how far from one to the next, how wide its bend to a dot is, and how
/// much of a line a mark takes, above and below its middle.
const CURL_X: f32 = 20.0;
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
/// How strong what is drawn for a section's steps is while the section
/// is not being worked on, and how long it takes to come forward.
const BACK: f32 = 0.38;
const FORWARD: f32 = 0.16;

/// Something that is drawn in when it first appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Thing {
    Line(StepId),
    Loop(StepId, StepId),
    Order(StepId, StepId),
    Again(StepId),
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
    /// The section whose steps the pointer is on: `Some(None)` on steps
    /// in no section.
    among: Option<Option<StepId>>,
    /// How far forward each section's steps have come.
    forward: HashMap<Option<StepId>, f32>,
}

impl Moves {
    /// Moves everything on by `passed` seconds towards what `picture`
    /// shows; whether anything is still on its way.
    fn on(&mut self, picture: &Picture, passed: f32) -> bool {
        let fresh = !self.begun || picture.still;
        self.begun = true;
        let mut moving = false;
        // What has played.
        let playing = picture.items.iter().find_map(|item| match item {
            Item::Stop(Stop {
                id,
                line: Some(Line::Playing { done, .. }),
                ..
            }) => Some((*id, *done)),
            _ => None,
        });
        self.played = playing.map(|(id, done)| {
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
        let on = picture.items.iter().find_map(|item| match item {
            Item::Stop(Stop {
                id,
                dot: Dot::On { .. },
                ..
            }) => Some(*id),
            _ => None,
        });
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
        for item in &picture.items {
            if let Item::Stop(Stop {
                id, line: Some(_), ..
            }) = item
            {
                draw(Thing::Line(*id), false);
            }
        }
        for beside in &picture.loops {
            draw(Thing::Loop(beside.first, beside.last), beside.pulled);
        }
        for beside in &picture.orders {
            draw(Thing::Order(beside.first, beside.last), beside.pulled);
        }
        for section in &picture.sections {
            if section.again.is_some() && section.last.is_some() {
                draw(Thing::Again(section.id), false);
            }
        }
        self.drawn = drawn;
        // The steps of a section that is worked on come forward: one of
        // them is on, selected or pulled, or the pointer is on them.
        let mut forward = HashMap::new();
        for item in &picture.items {
            let Item::Stop(stop) = item else {
                continue;
            };
            let wanted = stop.awake || self.among == Some(stop.group);
            let entry = forward.entry(stop.group).or_insert((false, 0.0));
            entry.0 |= wanted;
        }
        for (group, (wanted, far)) in &mut forward {
            let target = if *wanted { 1.0 } else { 0.0 };
            let from = match self.forward.get(group) {
                Some(far) if !fresh => *far,
                _ => target,
            };
            let step = passed / FORWARD;
            *far = match target > from {
                true => (from + step).min(target),
                false => (from - step).max(target),
            };
            moving |= *far != target;
        }
        self.forward = forward
            .into_iter()
            .map(|(group, (_, far))| (group, far))
            .collect();
        moving
    }

    /// How strong what is drawn for the steps of `group` is.
    fn strength(&self, group: Option<StepId>) -> f32 {
        let far = self.forward.get(&group).copied().unwrap_or(1.0);
        BACK + (1.0 - BACK) * ease(far)
    }

    fn drawn(&self, thing: Thing) -> f32 {
        ease(self.drawn.get(&thing).copied().unwrap_or(1.0))
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
    HashMap<Option<StepId>, f32>,
);

#[derive(Default)]
struct State {
    kept: Kept<Shown>,
    moves: Moves,
}

struct Over {
    picture: Picture,
    places: Places,
}

impl<Message> canvas::Program<Message> for Over {
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
                let moving = state.moves.on(&self.picture, passed);
                state.moves.last = moving.then_some(*now);
                moving.then(canvas::Action::request_redraw)
            }
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let over = cursor
                    .position()
                    .filter(|_| self.picture.hot && cursor.is_over(bounds))
                    .and_then(|at| {
                        let places = self.places.borrow();
                        self.picture.items.iter().find_map(|item| match item {
                            Item::Stop(stop) => places
                                .get(&Key::Dot(stop.id))
                                .filter(|dot| dot.center().distance(at) <= DOT + RING)
                                .map(|_| stop.id),
                            Item::Section { .. } => None,
                        })
                    });
                // The section whose steps it is among: level with one of
                // their rows.
                let among = cursor
                    .position()
                    .filter(|_| cursor.is_over(bounds))
                    .and_then(|at| {
                        let places = self.places.borrow();
                        self.picture.items.iter().find_map(|item| match item {
                            Item::Stop(stop) => places
                                .get(&Key::Row(stop.id))
                                .filter(|row| (row.y..row.y + row.height).contains(&at.y))
                                .map(|_| stop.group),
                            Item::Section { .. } => None,
                        })
                    });
                (over != state.moves.over || among != state.moves.among).then(|| {
                    state.moves.over = over;
                    state.moves.among = among;
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
        let places = self.places.borrow().clone();
        let moves = &state.moves;
        let mut drawn: Vec<(Thing, f32)> = moves
            .drawn
            .iter()
            .filter(|(_, far)| **far < 1.0)
            .map(|(thing, far)| (*thing, *far))
            .collect();
        drawn.sort_by(|a, b| a.1.total_cmp(&b.1));
        let shown = (
            self.picture.clone(),
            places.clone(),
            moves.played,
            moves.ring,
            drawn,
            moves.over,
            moves.forward.clone(),
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
        })]
    }
}

/// The canvas that draws `picture` over the list its rows are in: laid
/// over all of it, it takes no press and no pointer from what is under.
pub fn over<'a, Message: 'a>(picture: Picture, places: &Places) -> Element<'a, Message> {
    iced::widget::canvas(Over {
        picture,
        places: places.clone(),
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
fn part(frame: &mut Frame, run: &Run, from: f32, to: f32, color: Color, hollow: bool) {
    if to - from <= 0.0 {
        return;
    }
    let (a, b) = (along(run, from), along(run, to));
    if !hollow {
        let (corner, size) = bar(a, b, LINE);
        frame.fill_rectangle(corner, size, color);
        return;
    }
    let across = (run.1.x - run.0.x).abs() > (run.1.y - run.0.y).abs();
    for side in [-1.0, 1.0] {
        let off = side * (LINE - SIDE) / 2.0;
        let shift = match across {
            true => Vector::new(0.0, off),
            false => Vector::new(off, 0.0),
        };
        let (corner, size) = bar(a + shift, b + shift, SIDE);
        frame.fill_rectangle(corner, size, color);
    }
}

/// The way from the dot of the stop at `n` to the next one's, in
/// straight parts: round the dot of any section it passes, and no
/// further than a cut lets it be seen.
fn way(picture: &Picture, n: usize, at: &dyn Fn(Key) -> Option<Rectangle>) -> Vec<Run> {
    let Item::Stop(stop) = &picture.items[n] else {
        return Vec::new();
    };
    let Some(dot) = at(Key::Dot(stop.id)) else {
        return Vec::new();
    };
    let limit = |cut: Option<u32>| {
        cut.and_then(|cut| at(Key::Cut(cut)))
            .map_or(f32::INFINITY, |cut| cut.y + cut.height)
    };
    let x = dot.center_x();
    let mut y = dot.center_y() + stop.dot.reach();
    let mut runs: Vec<Run> = Vec::new();
    let down = |runs: &mut Vec<Run>, x: f32, from: f32, to: f32| {
        if to > from {
            runs.push((Point::new(x, from), Point::new(x, to)));
        }
    };
    let later = match stop.next {
        Some(_) => &picture.items[n + 1..],
        None => &[][..],
    };
    for later in later {
        match later {
            Item::Section { id, cut } => {
                let Some(section) = at(Key::Section(*id)) else {
                    continue;
                };
                let to = section.y - ROOM;
                down(&mut runs, x, y, to.min(limit(*cut)));
                y = section.y + section.height + ROOM;
                if to > limit(*cut) {
                    return runs;
                }
            }
            // The step it goes to: down to its dot, across first where
            // that one is further in or out.
            Item::Stop(next) if Some(next.id) == stop.next => {
                let Some(there) = at(Key::Dot(next.id)) else {
                    return runs;
                };
                let to = (there.center_y() - next.dot.reach()).min(limit(next.cut));
                let over = there.center_x();
                if (over - x).abs() > 0.5 && runs.is_empty() {
                    let turn = (to - BEND * 2.0).max(y);
                    down(&mut runs, x, y, turn);
                    runs.push((Point::new(x, turn), Point::new(over, turn)));
                    down(&mut runs, over, turn, to);
                } else {
                    down(&mut runs, over, y, to);
                }
                return runs;
            }
            // Its own next step is folded away: as far as the heading it
            // is under.
            Item::Stop(_) => {
                runs.truncate(1);
                return runs;
            }
        }
    }
    // Nothing under it: to the foot of its own row.
    match runs.is_empty() {
        true => {
            if let Some(row) = at(Key::Row(stop.id)) {
                down(
                    &mut runs,
                    x,
                    y,
                    (row.y + row.height - ROOM).min(limit(stop.cut)),
                );
            }
        }
        false => runs.truncate(1),
    }
    runs
}

fn paint(
    frame: &mut Frame,
    picture: &Picture,
    at: &dyn Fn(Key) -> Option<Rectangle>,
    moves: &Moves,
) {
    let limit = |cut: Option<u32>| {
        cut.and_then(|cut| at(Key::Cut(cut)))
            .map_or(f32::INFINITY, |cut| cut.y + cut.height)
    };
    // The lines from dot to dot.
    for (n, item) in picture.items.iter().enumerate() {
        let Item::Stop(stop) = item else {
            continue;
        };
        let Some(line) = stop.line else {
            continue;
        };
        let runs = way(picture, n, at);
        let whole: f32 = runs.iter().map(long).sum();
        // As much of it as is drawn so far.
        let shown = whole * moves.drawn(Thing::Line(stop.id));
        let (played, ink, ahead) = match line {
            Line::Plain(color) => {
                let color = faded(color, moves.strength(stop.group));
                (0.0, color, color)
            }
            Line::Playing {
                done,
                played,
                ahead,
            } => {
                let done = match moves.played {
                    Some((id, shown)) if id == stop.id => shown,
                    _ => done,
                };
                (whole * done.clamp(0.0, 1.0), played, ahead)
            }
        };
        let hollow = matches!(line, Line::Playing { .. });
        let mut before = 0.0;
        for run in &runs {
            let length = long(run);
            let end = (shown - before).min(length);
            let solid = (played - before).clamp(0.0, length).min(end.max(0.0));
            part(frame, run, 0.0, solid, ink, false);
            part(frame, run, solid, end, ahead, hollow);
            before += length;
        }
    }
    // The sections' own lines.
    for section in &picture.sections {
        let Some(dot) = at(Key::Section(section.id)) else {
            continue;
        };
        let x = dot.center_x();
        let top = dot.y + dot.height + ROOM;
        let foot = limit(section.cut);
        let end = section
            .last
            .and_then(|last| at(Key::Dot(last)))
            .map(|last| last.center_y());
        // On to the next section's dot, where it hands over.
        let next = section
            .next
            .and_then(|next| at(Key::Section(next)))
            .filter(|next| (next.center_x() - x).abs() < 0.5)
            .map(|next| next.y - ROOM);
        let again = section.again.as_ref().and(end);
        let down = match (next, again) {
            (Some(next), _) => Some(next),
            (None, Some(end)) => Some(end),
            (None, None) => None,
        };
        if let Some(down) = down.map(|down| down.min(foot))
            && down > top
        {
            part(
                frame,
                &(Point::new(x, top), Point::new(x, down)),
                0.0,
                down - top,
                section.color,
                false,
            );
        }
        // Where it comes again: back from the level of its last step.
        if let (Some(mark), Some(end)) = (&section.again, end)
            && end < foot
            && picture.curl_wide > 0.0
        {
            frame.fill(&Path::circle(Point::new(x, end), 3.0), section.color);
            turn(
                frame,
                section.color,
                moves.drawn(Thing::Again(section.id)),
                &Turn {
                    x: x - CURL_X,
                    top: dot.center_y(),
                    bottom: end,
                    from: end - BEND,
                    to: dot.center_y() + BEND,
                    out_from: x - 4.5,
                    tip: x - dot.width / 2.0 - 2.5,
                    starts: true,
                    ends: true,
                },
                mark,
            );
        }
    }
    // The lines beside them.
    let pulling = picture.loops.iter().any(|beside| beside.pulled);
    for beside in &picture.loops {
        let far = moves.drawn(Thing::Loop(beside.first, beside.last));
        // What is there steps back while another is pulled over it.
        let color = match (beside.pulled, pulling) {
            (true, _) => beside.color,
            (false, true) => faded(beside.color, 0.35),
            (false, false) => faded(beside.color, moves.strength(beside.group)),
        };
        curl(frame, picture, beside, color, far, at);
    }
    let pulling = picture.orders.iter().any(|beside| beside.pulled);
    for beside in &picture.orders {
        let far = moves.drawn(Thing::Order(beside.first, beside.last));
        let color = match (beside.pulled, pulling) {
            (true, _) => beside.color,
            (false, true) => faded(beside.color, 0.35),
            (false, false) => faded(beside.color, moves.strength(beside.group)),
        };
        wander(frame, picture, beside, color, far, at);
    }
    // The dots, over the ends of the lines.
    for item in &picture.items {
        let Item::Stop(stop) = item else {
            continue;
        };
        let Some(place) = at(Key::Dot(stop.id)) else {
            continue;
        };
        let middle = place.center();
        // Under a cut, only as much of it as the cut leaves.
        let room = limit(stop.cut) - (middle.y - stop.dot.reach());
        if room <= 0.0 {
            continue;
        }
        let whole = room >= stop.dot.reach() * 2.0;
        let draw = |frame: &mut Frame| {
            if moves.over == Some(stop.id) {
                let color = stop.dot.color();
                frame.fill(
                    &Path::circle(middle, DOT + RING),
                    Color {
                        a: color.a * 0.22,
                        ..color
                    },
                );
            }
            dot(frame, stop, middle, moves);
        };
        match whole {
            true => draw(frame),
            false => frame.with_clip(
                Rectangle::new(
                    Point::new(middle.x - 30.0, middle.y - stop.dot.reach() - 12.0),
                    Size::new(60.0, room + 12.0),
                ),
                |frame| {
                    // A clipped frame starts at its own corner.
                    frame.translate(Vector::new(
                        30.0 - middle.x,
                        stop.dot.reach() + 12.0 - middle.y,
                    ));
                    draw(frame);
                },
            ),
        }
    }
}

fn dot(frame: &mut Frame, stop: &Stop, middle: Point, moves: &Moves) {
    let glow = |frame: &mut Frame, color: Color, gone: f32| {
        frame.stroke(
            &Path::circle(middle, DOT + 1.5 + 9.0 * gone),
            Stroke::default().with_width(2.5).with_color(Color {
                a: color.a * 0.6 * (1.0 - gone),
                ..color
            }),
        );
    };
    match stop.dot {
        Dot::Solid(color) => frame.fill(
            &Path::circle(middle, DOT),
            faded(color, moves.strength(stop.group)),
        ),
        Dot::Hollow(color) => frame.stroke(
            &Path::circle(middle, DOT - 1.0),
            Stroke::default().with_width(2.0).with_color(color),
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
                Stroke::default().with_width(1.5).with_color(Color {
                    a: ring.a * open,
                    ..ring
                }),
            );
            frame.fill(&Path::circle(middle, DOT), color);
        }
    }
}

/// `color`, `strength` as strong.
fn faded(color: Color, strength: f32) -> Color {
    Color {
        a: color.a * strength,
        ..color
    }
}

/// Dashes along the straight line from `from` to `to`, as many as fit,
/// each in the middle of its share: half a gap at either end. Only those
/// that end within `upto` of its start are drawn.
fn dashes(frame: &mut Frame, stroke: Stroke<'_>, from: Point, to: Point, upto: f32) {
    let run = (from, to);
    let whole = long(&run);
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

/// The line of what comes again: out from its last step's dot, up beside
/// the steps, and in to its first step's with an arrow's head. `far` of
/// it is drawn, from where it leaves the last step.
fn curl(
    frame: &mut Frame,
    picture: &Picture,
    beside: &Beside,
    color: Color,
    far: f32,
    at: &dyn Fn(Key) -> Option<Rectangle>,
) {
    let (Some(first), Some(last)) = (at(Key::Dot(beside.first)), at(Key::Dot(beside.last))) else {
        return;
    };
    let (Some(first_row), Some(last_row)) = (at(Key::Row(beside.first)), at(Key::Row(beside.last)))
    else {
        return;
    };
    // Down the room the rows keep for it: the one furthest out, where
    // they are not all as far in.
    let x = first_row.x.min(last_row.x) + CURL_X.min(picture.curl_wide / 2.0);
    let (top, bottom) = (first.center_y(), last.center_y());
    // Where it turns, or runs on out of sight where its own end is
    // folded away.
    turn(
        frame,
        color,
        far,
        &Turn {
            x,
            top,
            bottom,
            from: match beside.ends {
                true => bottom - BEND,
                false => last_row.y + last_row.height,
            },
            to: match beside.starts {
                true => top + BEND,
                false => first_row.y,
            },
            out_from: last.center_x() - DOT - RING - 1.0,
            tip: first.center_x() - DOT - RING - 2.0,
            starts: beside.starts,
            ends: beside.ends,
        },
        &beside.mark,
    );
}

/// Where a line that curls back runs.
struct Turn {
    /// Where it runs up.
    x: f32,
    /// The level of what it comes back to, and of what it leaves.
    top: f32,
    bottom: f32,
    /// Where its run up starts and ends.
    from: f32,
    to: f32,
    /// Where it leaves from, and where its arrow's head points to.
    out_from: f32,
    tip: f32,
    /// Whether it really starts and ends there, with its bends.
    starts: bool,
    ends: bool,
}

/// The line `turn` describes, `far` of it drawn from where it leaves,
/// with `mark` in a gap at the middle of its run up.
fn turn(frame: &mut Frame, color: Color, far: f32, turn: &Turn, mark: &Mark) {
    let Turn {
        x,
        top,
        bottom,
        from,
        to,
        ..
    } = *turn;
    let stroke = Stroke {
        line_cap: canvas::LineCap::Round,
        line_join: canvas::LineJoin::Round,
        ..Stroke::default().with_color(color).with_width(1.5)
    };
    let out = (turn.out_from - (x + BEND + 1.0)).max(0.0);
    let into = (turn.tip - 4.0 - (x + BEND + 1.0)).max(0.0);
    let up = (from - to).max(0.0);
    let whole = out + up + into;
    let mut left = whole * far;
    // Out from the last step's dot.
    if turn.ends {
        dashes(
            frame,
            stroke,
            Point::new(x + BEND + 1.0 + out, bottom),
            Point::new(x + BEND + 1.0, bottom),
            left,
        );
        if left >= out {
            frame.stroke(
                &Path::new(|path| {
                    path.move_to(Point::new(x + BEND, bottom));
                    path.quadratic_curve_to(Point::new(x, bottom), Point::new(x, bottom - BEND));
                }),
                stroke,
            );
        }
        left -= out;
    }
    // Up, round what it says of itself at its middle.
    let words = match mark {
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
            );
            gone += from - to;
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
    // In to the first step's dot, with the arrow's head.
    if turn.starts && left > 0.0 {
        frame.stroke(
            &Path::new(|path| {
                path.move_to(Point::new(x, top + BEND));
                path.quadratic_curve_to(Point::new(x, top), Point::new(x + BEND, top));
            }),
            stroke,
        );
        dashes(
            frame,
            stroke,
            Point::new(x + BEND + 1.0, top),
            Point::new(x + BEND + 1.0 + into, top),
            left,
        );
        if left >= into {
            let tip = x + BEND + 1.0 + into + 4.0;
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
    picture: &Picture,
    beside: &Beside,
    color: Color,
    far: f32,
    at: &dyn Fn(Key) -> Option<Rectangle>,
) {
    let (Some(first), Some(last)) = (at(Key::Dot(beside.first)), at(Key::Dot(beside.last))) else {
        return;
    };
    let (Some(first_row), Some(last_row)) = (at(Key::Row(beside.first)), at(Key::Row(beside.last)))
    else {
        return;
    };
    let stroke = Stroke {
        line_cap: canvas::LineCap::Round,
        line_join: canvas::LineJoin::Round,
        ..Stroke::default().with_color(color).with_width(1.5)
    };
    let x =
        (first_row.x + first_row.width).max(last_row.x + last_row.width) - picture.order_wide / 2.0;
    let (top, bottom) = (
        match beside.starts {
            true => first.center_y(),
            false => first_row.y,
        },
        match beside.ends {
            true => last.center_y(),
            false => last_row.y + last_row.height,
        },
    );
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
    if beside.mark != Mark::Crossed || whole < MARK_ROOM * 3.0 {
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

    fn stop(n: u64, line: Option<Line>, next: Option<u64>) -> Item {
        Item::Stop(Stop {
            id: id(n),
            dot: Dot::Solid(Color::WHITE),
            line,
            next: next.map(id),
            group: None,
            awake: true,
            cut: None,
        })
    }

    /// Dots 10 across, their middles at x 25 and 40 apart down from 20.
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
            items: vec![
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
    fn a_line_leaves_room_for_the_dot_of_a_section_it_passes() {
        let picture = Picture {
            items: vec![
                stop(1, Some(Line::Plain(Color::WHITE)), Some(3)),
                Item::Section {
                    id: id(2),
                    cut: None,
                },
                stop(3, None, None),
            ],
            ..Picture::default()
        };
        let at = places([
            (Key::Dot(id(1)), 20.0),
            (Key::Section(id(2)), 60.0),
            (Key::Dot(id(3)), 100.0),
        ]);
        let runs = way(&picture, 0, &at);
        assert_eq!(
            runs,
            vec![
                (Point::new(25.0, 25.0), Point::new(25.0, 53.0)),
                (Point::new(25.0, 67.0), Point::new(25.0, 95.0)),
            ]
        );
    }

    #[test]
    fn the_last_steps_line_ends_at_the_foot_of_its_row() {
        let picture = Picture {
            items: vec![stop(1, Some(Line::Plain(Color::WHITE)), None)],
            ..Picture::default()
        };
        let at = places([(Key::Dot(id(1)), 20.0), (Key::Row(id(1)), 20.0)]);
        let runs = way(&picture, 0, &at);
        assert_eq!(runs, vec![(Point::new(25.0, 25.0), Point::new(25.0, 38.0))]);
    }

    #[test]
    fn a_line_stops_at_the_heading_its_next_step_is_folded_under() {
        let picture = Picture {
            items: vec![
                stop(1, Some(Line::Plain(Color::WHITE)), Some(9)),
                Item::Section {
                    id: id(2),
                    cut: None,
                },
                stop(3, None, None),
            ],
            ..Picture::default()
        };
        let at = places([
            (Key::Dot(id(1)), 20.0),
            (Key::Section(id(2)), 60.0),
            (Key::Dot(id(3)), 100.0),
        ]);
        let runs = way(&picture, 0, &at);
        assert_eq!(runs, vec![(Point::new(25.0, 25.0), Point::new(25.0, 53.0))]);
    }

    #[test]
    fn what_has_played_catches_up_and_a_new_line_is_drawn_in() {
        let playing = |done| Picture {
            items: vec![stop(
                1,
                Some(Line::Playing {
                    done,
                    played: Color::WHITE,
                    ahead: Color::BLACK,
                }),
                None,
            )],
            ..Picture::default()
        };
        let mut moves = Moves::default();
        // What is there at first is there.
        assert!(!moves.on(&playing(0.25), 0.0));
        assert_eq!(moves.played, Some((id(1), 0.25)));
        assert_eq!(moves.drawn(Thing::Line(id(1))), 1.0);
        // A jump is caught up with, not made at once.
        assert!(moves.on(&playing(0.75), 0.05));
        let (_, shown) = moves.played.unwrap();
        assert!(shown > 0.25 && shown < 0.75);
        for _ in 0..100 {
            moves.on(&playing(0.75), 0.05);
        }
        assert_eq!(moves.played, Some((id(1), 0.75)));
        // A line that was not there is drawn in.
        let mut two = playing(0.75);
        two.items
            .push(stop(2, Some(Line::Plain(Color::WHITE)), None));
        assert!(moves.on(&two, 0.05));
        assert!(moves.drawn(Thing::Line(id(2))) < 1.0);
        // With motion off it is there at once.
        two.still = true;
        two.items
            .push(stop(3, Some(Line::Plain(Color::WHITE)), None));
        assert!(!moves.on(&two, 0.05));
        assert_eq!(moves.drawn(Thing::Line(id(3))), 1.0);
    }
}
