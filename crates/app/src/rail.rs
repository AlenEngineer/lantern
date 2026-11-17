//! The rail by hand: a step's dot pulled to another step's.
//!
//! Pulled up, a loop is drawn from the step it was taken from back to
//! the one it is let go on: they come again. Pulled down, the steps
//! between go on by themselves, one into the next. Let go where it was
//! pressed, it is a click: the step goes on, or waits.
//!
//! A right click on a dot, on the line between two, or on a loop opens
//! a ring of what can be done to that one thing: only what fits it. A
//! step can come again but has no order to draw; a loop has both.

use iced::widget::{Space, button, container, mouse_area, pin, text, tooltip};
use iced::{Element, Fill, Point};
use lantern_engine::{Plan, Repeat, StepId};

use crate::look::{self, BOLD, Icon, META};
use crate::widgets::hinted;
use crate::{App, Message};

/// What is done on the rail.
#[derive(Debug, Clone)]
pub enum Act {
    /// A dot was pressed. `toggle` is what a click there would make of
    /// whether the step goes on by itself, where a click may.
    Press {
        step: StepId,
        toggle: Option<bool>,
    },
    /// The pointer came onto a step's rail, or left it.
    Over(StepId),
    Left(StepId),
    /// A right click: the ring of what can be done to that thing.
    Ring(RingOn),
    RingClose,
    /// Takes a loop away: one drawn round some steps, or how often a
    /// section or the plan (`None`) comes.
    Unloop(Option<StepId>),
}

/// What a ring is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingOn {
    /// A step, by its dot.
    Dot(StepId),
    /// The line from a step to the next.
    Line(StepId),
    /// What repeats: a loop, a section, or the plan (`None`).
    Loop(Option<StepId>),
}

/// A ring that is open, and where: round the pointer as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    pub on: RingOn,
    pub at: Point,
}

/// How big a ring's buttons are, and the ones of its outer row.
const PETAL: f32 = 34.0;
const COUNT: f32 = 30.0;

impl App {
    /// The ring that is open, as layers over the dashboard: something
    /// behind it to press to close it, then each of its buttons where it
    /// goes. Empty when none is open, or what it was about is gone.
    pub(crate) fn ring_layers(&self) -> Vec<Element<'_, Message>> {
        let Some(ring) = self.ring else {
            return Vec::new();
        };
        let p = self.palette;
        let plan = self.engine.0.plan();
        // They fan out from the middle as it opens.
        let out = self.motion.lift();
        let close = Message::Rail(Act::RingClose);
        let mut layers: Vec<Element<'_, Message>> = vec![
            mouse_area(Space::new().width(Fill).height(Fill))
                .on_press(close.clone())
                .on_right_press(close)
                .into(),
        ];
        // Kept on the window: near its top or foot the ring moves in.
        let tall = self.dashboard_size.height;
        let middle = Point::new(ring.at.x, ring.at.y.clamp(120.0, (tall - 120.0).max(120.0)));
        let mut petals: Vec<Petal> = Vec::new();
        let ink = |lit: bool| if lit { p.on_primary } else { p.on_surface };
        let glyph = |icon: Icon, lit: bool| -> Element<'static, Message> {
            look::icon(icon, 16.0, ink(lit)).into()
        };
        let word = |words: &'static str, lit: bool| -> Element<'static, Message> {
            text(words).size(META).font(BOLD).color(ink(lit)).into()
        };
        // How often: an outer row of the ring, round the repeat button.
        let counts = |of: Option<StepId>, repeat: Repeat, from: f32, far: f32| -> Vec<Petal> {
            let mut petals = Vec::new();
            let choices: [(&'static str, Repeat, &str); 3] = [
                ("×2", Repeat::Times(2), "Twice"),
                ("×3", Repeat::Times(3), "Three times"),
                ("∞", Repeat::Forever, "Until you move on"),
            ];
            for (n, (words, choice, about)) in choices.into_iter().enumerate() {
                let lit = repeat == choice;
                petals.push(petal(
                    from + 20.0 * n as f32,
                    far,
                    COUNT,
                    lit,
                    word(words, lit),
                    about,
                    Some(Message::SetRepeat(of, choice)),
                ));
            }
            // Another number, or until a time: typed where the choices are.
            let other = !matches!(
                repeat,
                Repeat::Once | Repeat::Times(2 | 3) | Repeat::Forever
            );
            petals.push(petal(
                from + 60.0,
                far,
                COUNT,
                other,
                glyph(Icon::Clock, other),
                "Another number of times, or until a time",
                Some(Message::RepeatPanel(of)),
            ));
            petals
        };
        match ring.on {
            RingOn::Dot(id) => {
                let Some(at) = plan.position(id) else {
                    return Vec::new();
                };
                let step = &plan.steps()[at];
                let goes = plan.goes_on(at);
                let again = step.flow.repeat != Repeat::Once;
                petals.push(petal(
                    -60.0,
                    46.0,
                    PETAL,
                    goes,
                    glyph(Icon::Joined, goes),
                    if goes {
                        "Goes straight on. Press to make it wait"
                    } else {
                        "Go straight on to the next"
                    },
                    crate::plan_edit::goes_on_when(step)
                        .is_some()
                        .then_some(Message::SetAdvance(id, !goes)),
                ));
                petals.push(petal(
                    0.0,
                    46.0,
                    PETAL,
                    again,
                    glyph(Icon::Repeat, again),
                    if again {
                        "Comes again. Press for once"
                    } else {
                        "Come again"
                    },
                    Some(Message::SetRepeat(
                        Some(id),
                        if again {
                            Repeat::Once
                        } else {
                            Repeat::Times(2)
                        },
                    )),
                ));
                petals.push(petal(
                    60.0,
                    46.0,
                    PETAL,
                    false,
                    word("✕", false),
                    "Take it out of the plan",
                    Some(Message::Remove(id)),
                ));
                if again {
                    petals.extend(counts(Some(id), step.flow.repeat, -30.0, 90.0));
                }
            }
            RingOn::Line(id) => {
                let Some(at) = plan.position(id) else {
                    return Vec::new();
                };
                let goes = plan.goes_on(at);
                petals.push(petal(
                    -30.0,
                    44.0,
                    PETAL,
                    goes,
                    glyph(Icon::Joined, goes),
                    "Goes straight on",
                    Some(Message::SetAdvance(id, true)),
                ));
                petals.push(petal(
                    30.0,
                    44.0,
                    PETAL,
                    !goes,
                    glyph(Icon::Pause, !goes),
                    "Waits here",
                    Some(Message::SetAdvance(id, false)),
                ));
            }
            RingOn::Loop(of) => {
                let (flow, all_go) = match of {
                    Some(id) => {
                        let Some(step) = plan.get(id) else {
                            return Vec::new();
                        };
                        (step.flow, step.advance == Some(true))
                    }
                    None => (plan.flow, plan.advance),
                };
                let again = flow.repeat != Repeat::Once;
                petals.push(petal(
                    -72.0,
                    50.0,
                    PETAL,
                    again,
                    glyph(Icon::Repeat, again),
                    "Comes again",
                    Some(Message::SetRepeat(
                        of,
                        if again {
                            Repeat::Once
                        } else {
                            Repeat::Times(2)
                        },
                    )),
                ));
                petals.push(petal(
                    -24.0,
                    50.0,
                    PETAL,
                    flow.shuffle,
                    glyph(Icon::Shuffle, flow.shuffle),
                    "In an order drawn afresh each time",
                    Some(Message::SetShuffle(of, !flow.shuffle)),
                ));
                petals.push(petal(
                    24.0,
                    50.0,
                    PETAL,
                    all_go,
                    glyph(Icon::Joined, all_go),
                    "All of it goes on by itself",
                    Some(match of {
                        Some(id) => Message::SetAdvance(id, !all_go),
                        None => Message::PlanAdvance(!all_go),
                    }),
                ));
                petals.push(petal(
                    72.0,
                    50.0,
                    PETAL,
                    false,
                    word("✕", false),
                    "No loop",
                    Some(Message::Rail(Act::Unloop(of))),
                ));
                if again {
                    petals.extend(counts(of, flow.repeat, -78.0, 98.0));
                }
            }
        }
        for petal in petals {
            let Petal {
                angle,
                far,
                size,
                lit,
                face,
                about,
                message,
            } = petal;
            let (sin, cos) = angle.to_radians().sin_cos();
            let far = far * out;
            let ground = match lit {
                true => p.primary,
                false => crate::motion::blend(p.surface_container_highest, p.on_surface, 0.1),
            };
            let round = button(container(face).center_x(size).center_y(size))
                .width(size)
                .height(size)
                .padding(0)
                .on_press_maybe(message)
                .style(move |_, status| button::Style {
                    background: Some(
                        match status {
                            button::Status::Hovered | button::Status::Pressed => {
                                crate::motion::blend(ground, p.on_surface, 0.15)
                            }
                            _ => ground,
                        }
                        .scale_alpha(out)
                        .into(),
                    ),
                    border: iced::Border {
                        radius: (size / 2.0).into(),
                        ..iced::Border::default()
                    },
                    shadow: iced::Shadow {
                        color: iced::Color::BLACK.scale_alpha(0.45 * out),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    },
                    ..button::Style::default()
                });
            layers.push(
                pin(hinted(p, round, about, tooltip::Position::Right))
                    .x(middle.x + far * cos - size / 2.0)
                    .y(middle.y + far * sin - size / 2.0)
                    .into(),
            );
        }
        layers
    }
}

/// One button of a ring: where it is round the middle (`angle` in
/// degrees from the right, `far` out), how big, whether it is lit, what
/// is on it, what it says of itself, and what pressing it does.
struct Petal {
    angle: f32,
    far: f32,
    size: f32,
    lit: bool,
    face: Element<'static, Message>,
    about: String,
    message: Option<Message>,
}

fn petal(
    angle: f32,
    far: f32,
    size: f32,
    lit: bool,
    face: Element<'static, Message>,
    about: &str,
    message: Option<Message>,
) -> Petal {
    Petal {
        angle,
        far,
        size,
        lit,
        face,
        about: about.to_string(),
        message,
    }
}

/// What letting go would do, to the rows from `first` to `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Would {
    /// A loop round them.
    Loop { first: usize, last: usize },
    /// Each goes on into the next.
    Join { first: usize, last: usize },
    /// Nothing: no loop can be drawn there.
    Not { first: usize, last: usize },
}

impl Would {
    /// The rows it is about.
    pub fn rows(self) -> (usize, usize) {
        match self {
            Would::Loop { first, last }
            | Would::Join { first, last }
            | Would::Not { first, last } => (first, last),
        }
    }
}

/// A dot held down.
#[derive(Debug, Clone, PartialEq)]
pub struct Pull {
    pub from: StepId,
    /// What a click would do: see [`Act::Press`].
    pub toggle: Option<bool>,
    origin: Option<Point>,
    /// Where the pointer is, in the dashboard window.
    pub at: Point,
    /// Whether it has moved far enough to be a pull, not a click.
    pub moving: bool,
    /// What letting go now would do.
    pub would: Option<Would>,
}

/// How far the pointer moves before a press becomes a pull.
const SLOP: f32 = 5.0;

impl Pull {
    pub fn new(from: StepId, toggle: Option<bool>) -> Pull {
        Pull {
            from,
            toggle,
            origin: None,
            at: Point::ORIGIN,
            moving: false,
            would: None,
        }
    }

    /// The pointer moved to `at`.
    pub fn to(&mut self, at: Point) {
        let origin = *self.origin.get_or_insert(at);
        self.at = at;
        if origin.distance(at) > SLOP {
            self.moving = true;
        }
    }

    /// Works out what letting go over `over` would do.
    pub fn aim(&mut self, plan: &Plan, over: Option<StepId>) {
        self.would = self
            .moving
            .then_some(over)
            .flatten()
            .filter(|over| *over != self.from)
            .and_then(|over| {
                let (from, to) = (plan.position(self.from)?, plan.position(over)?);
                if plan.steps()[to].is_section() {
                    return None;
                }
                Some(if to > from {
                    Would::Join {
                        first: from,
                        last: to,
                    }
                } else if plan.clone().loop_over(self.from, over).is_some() {
                    Would::Loop {
                        first: to,
                        last: from,
                    }
                } else {
                    Would::Not {
                        first: to,
                        last: from,
                    }
                })
            });
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::Action;

    use super::*;

    #[test]
    fn up_is_a_loop_down_is_a_join_and_across_a_heading_is_neither() {
        let mut plan = Plan::new("Sabbath");
        plan.section("Music", None);
        let [a, b, c] = ["A", "B", "C"].map(|name| plan.push(name, Action::Blank));
        plan.section("Sermon", None);
        let d = plan.push("D", Action::Blank);
        let mut pull = Pull::new(c, Some(true));
        pull.to(Point::new(10.0, 100.0));
        pull.aim(&plan, Some(a));
        assert_eq!(pull.would, None, "not moved yet: a click");
        pull.to(Point::new(10.0, 40.0));
        pull.aim(&plan, Some(a));
        assert_eq!(pull.would, Some(Would::Loop { first: 1, last: 3 }));
        pull.aim(&plan, Some(c));
        assert_eq!(pull.would, None, "back on itself");
        pull.aim(&plan, None);
        assert_eq!(pull.would, None);
        // From A down to C: A and B go on.
        let mut pull = Pull::new(a, None);
        pull.to(Point::new(10.0, 40.0));
        pull.to(Point::new(10.0, 100.0));
        pull.aim(&plan, Some(c));
        assert_eq!(pull.would, Some(Would::Join { first: 1, last: 3 }));
        // From D up to B: across Sermon's heading.
        let mut pull = Pull::new(d, None);
        pull.to(Point::new(10.0, 140.0));
        pull.to(Point::new(10.0, 60.0));
        pull.aim(&plan, Some(b));
        assert_eq!(pull.would, Some(Would::Not { first: 2, last: 5 }));
    }
}
