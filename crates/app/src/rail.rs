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
    /// How something that comes again until a time ends when it comes.
    Ends(Option<StepId>, lantern_engine::Ending),
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
/// How tall the pills are that say how often.
const PILL: f32 = 30.0;

impl App {
    /// How often `of` comes, beside the ring's repeat button: a number
    /// of times with fewer and more, until you move on, or until a time
    /// of day typed there. With a time, under it: how it ends when the
    /// time comes, and what that comes to.
    fn how_often<'a>(
        &self,
        plan: &Plan,
        of: Option<StepId>,
        flow: lantern_engine::Flow,
    ) -> Element<'a, Message> {
        use lantern_engine::Ending;
        let p = self.palette;
        let ground = move |lit: bool| match lit {
            true => p.primary,
            false => crate::motion::blend(p.surface_container_highest, p.on_surface, 0.1),
        };
        let ink = move |lit: bool| if lit { p.on_primary } else { p.on_surface };
        let pill = move |lit: bool| {
            move |_: &iced::Theme| container::Style {
                background: Some(ground(lit).into()),
                border: iced::Border {
                    radius: (PILL / 2.0).into(),
                    ..iced::Border::default()
                },
                shadow: iced::Shadow {
                    color: iced::Color::BLACK.scale_alpha(0.45),
                    offset: iced::Vector::new(0.0, 2.0),
                    blur_radius: 8.0,
                },
                ..container::Style::default()
            }
        };
        // A button inside a pill: no ground of its own.
        // Its words sit in the middle of its room, `wide` across, whatever
        // their shape: a minus is not a letter's height.
        let bare = move |lit: bool, words: String, wide: f32, size: f32, message: Message| {
            button(
                container(text(words).size(size).font(BOLD).color(ink(lit)))
                    .center_x(wide)
                    .center_y(PILL),
            )
            .padding(0)
            .width(wide)
            .height(PILL)
            .on_press(message)
            .style(move |_, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| p.on_surface.scale_alpha(0.12).into()),
                border: iced::Border {
                    radius: (PILL / 2.0).into(),
                    ..iced::Border::default()
                },
                ..button::Style::default()
            })
        };
        // A number of times, with fewer and more.
        let (times, counted) = match flow.repeat {
            Repeat::Times(times) => (times, true),
            _ => (2, false),
        };
        let set = |times: u16| Message::SetRepeat(of, Repeat::Times(times));
        let stepper = hinted(
            p,
            container(
                iced::widget::row![
                    bare(
                        counted,
                        "−".into(),
                        PILL,
                        look::BODY,
                        set(times.saturating_sub(1).max(2))
                    ),
                    bare(counted, format!("×{times}"), 30.0, META, set(times)),
                    bare(
                        counted,
                        "+".into(),
                        PILL,
                        look::BODY,
                        set(times.saturating_add(1).min(99))
                    ),
                ]
                .align_y(iced::Alignment::Center),
            )
            .style(pill(counted)),
            "A number of times".to_string(),
            tooltip::Position::Top,
        );
        let forever = flow.repeat == Repeat::Forever;
        let endless = hinted(
            p,
            container(bare(
                forever,
                "∞".into(),
                PILL,
                look::HEADING,
                Message::SetRepeat(of, Repeat::Forever),
            ))
            .style(pill(forever)),
            "Until you move on".to_string(),
            tooltip::Position::Top,
        );
        // A time of day, typed here: it holds once it reads as one.
        let time = match flow.repeat {
            Repeat::Until(time) => Some(time),
            _ => None,
        };
        let shown = match (&self.repeat_until, time) {
            (typed, _) if !typed.is_empty() => typed.clone(),
            (_, Some(time)) => time.to_string(),
            _ => String::new(),
        };
        let timed = time.is_some();
        let field = iced::widget::text_input("10:30", &shown)
            .on_input(Message::RepeatUntil)
            .size(META)
            .font(BOLD)
            .padding(0)
            .width(40)
            .style(move |_, _| iced::widget::text_input::Style {
                background: iced::Color::TRANSPARENT.into(),
                border: iced::Border::default(),
                icon: ink(timed),
                placeholder: ink(timed).scale_alpha(0.5),
                value: ink(timed),
                selection: p.on_surface.scale_alpha(0.3),
            });
        let until = hinted(
            p,
            container(
                iced::widget::row![look::icon(Icon::Clock, 14.0, ink(timed)), field]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
            )
            .padding([0, 10])
            .center_y(PILL)
            .style(pill(timed)),
            "Until a time of day, such as 10:30".to_string(),
            tooltip::Position::Top,
        );
        let mut rows = iced::widget::column![
            iced::widget::row![stepper, endless, until]
                .spacing(8)
                .align_y(iced::Alignment::Center)
        ]
        .spacing(6);
        let Some(time) = time else {
            return rows.into();
        };
        // How it ends when the time comes: there and then, with the step
        // that is on, or with the round.
        let ends = |ending: Ending, icon: Icon, about: &str| {
            let lit = flow.ends == ending;
            crate::widgets::place(
                p,
                look::icon(icon, 14.0, ink(lit)),
                lit,
                6,
                about,
                Message::Rail(Act::Ends(of, ending)),
            )
        };
        let ending = crate::widgets::places(
            p,
            [
                ends(
                    Ending::Sharp,
                    Icon::Stop,
                    "Ends at the time, cutting off what is on",
                ),
                ends(
                    Ending::Step,
                    Icon::SkipNext,
                    "Ends when the step that is on ends",
                ),
                ends(
                    Ending::Round,
                    Icon::Replay,
                    "Ends when once round is finished",
                ),
            ],
            PILL / 2.0,
        )
        .width(112);
        // What that comes to.
        let said = |words: String, colour: iced::Color, bold: bool| {
            text(words)
                .size(META)
                .font(if bold { BOLD } else { look::MEDIUM })
                .color(colour)
        };
        let lands = lantern_engine::timing::lands(plan, of, time, flow.ends, &|step| {
            self.loaded_length(step)
        });
        let mut readout = iced::widget::row![]
            .spacing(8)
            .align_y(iced::Alignment::Center);
        match lands.map(|lands| (lands.round, lands.ends)) {
            Some((_, Some(ends))) => {
                readout = readout
                    .push(said(format!("×{}", ends.rounds), p.primary, true))
                    .push(said(format!("→ {}", ends.at), p.on_surface, false));
                if ends.over >= std::time::Duration::from_secs(1) {
                    readout = readout.push(said(
                        format!("+{}", crate::length::clock(ends.over)),
                        p.tertiary,
                        true,
                    ));
                }
                if let Some((_, into)) = ends.cut {
                    readout = readout
                        .push(look::icon(Icon::Stop, 12.0, p.tertiary))
                        .push(said(crate::length::clock(into), p.tertiary, true));
                }
            }
            // No start time to count from: only how long once round is.
            Some((round, None)) => {
                readout = readout
                    .push(look::icon(Icon::Repeat, 12.0, p.on_surface_variant))
                    .push(said(crate::length::clock(round), p.on_surface, false))
                    .push(said("?".into(), p.on_surface_variant, true));
            }
            None => {}
        }
        let readout = container(readout)
            .padding([0, 12])
            .center_y(PILL)
            .style(move |_| container::Style {
                background: Some(p.surface_container_highest.into()),
                border: iced::Border {
                    radius: (PILL / 2.0).into(),
                    ..iced::Border::default()
                },
                ..container::Style::default()
            });
        rows = rows.push(
            iced::widget::row![ending, readout]
                .spacing(8)
                .align_y(iced::Alignment::Center),
        );
        rows.into()
    }

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
            // What is under the ring goes dim, so the ring is what reads.
            mouse_area(
                container(Space::new().width(Fill).height(Fill)).style(move |_| {
                    container::Style::default().background(p.surface.scale_alpha(0.6 * out))
                }),
            )
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
        // Where a button of the ring sits, from the ring's middle.
        let place = |angle: f32, far: f32| {
            let (sin, cos) = angle.to_radians().sin_cos();
            Point::new(middle.x + far * out * cos, middle.y + far * out * sin)
        };
        // What comes again, and where its button is: how often is set
        // beside it.
        let mut again_at: Option<(Option<StepId>, lantern_engine::Flow, Point)> = None;
        match ring.on {
            RingOn::Dot(id) => {
                let Some(at) = plan.position(id) else {
                    return Vec::new();
                };
                let step = &plan.steps()[at];
                let goes = plan.goes_on(at);
                let again = step.flow.repeat != Repeat::Once;
                petals.push(petal(
                    -30.0,
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
                    30.0,
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
                if again {
                    again_at = Some((Some(id), step.flow, place(30.0, 46.0)));
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
                let (flow, all_go, drawn) = match of {
                    Some(id) => {
                        let Some(step) = plan.get(id) else {
                            return Vec::new();
                        };
                        (step.flow, step.advance == Some(true), step.is_loop())
                    }
                    None => (plan.flow, plan.advance, false),
                };
                let again = flow.repeat != Repeat::Once;
                petals.push(petal(
                    -48.0,
                    50.0,
                    PETAL,
                    again,
                    glyph(Icon::Repeat, again),
                    if again {
                        "Comes again. Press for once"
                    } else {
                        "Come again"
                    },
                    // A loop drawn round some steps is nothing but that
                    // it comes again: once, it is gone.
                    Some(match (again, drawn) {
                        (true, true) => Message::Rail(Act::Unloop(of)),
                        (true, false) => Message::SetRepeat(of, Repeat::Once),
                        (false, _) => Message::SetRepeat(of, Repeat::Times(2)),
                    }),
                ));
                petals.push(petal(
                    0.0,
                    50.0,
                    PETAL,
                    flow.shuffle,
                    glyph(Icon::Shuffle, flow.shuffle),
                    "In an order drawn afresh each time",
                    Some(Message::SetShuffle(of, !flow.shuffle)),
                ));
                petals.push(petal(
                    48.0,
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
                if again {
                    again_at = Some((of, flow, place(-48.0, 50.0)));
                }
            }
        }
        if let Some((of, flow, at)) = again_at {
            layers.push(
                // Clear of the ring's buttons, level with the repeat one.
                pin(self.how_often(&plan, of, flow))
                    .x((at.x + PETAL / 2.0 + 8.0).max(middle.x + 50.0 + PETAL / 2.0 + 8.0))
                    .y(at.y - PILL / 2.0)
                    .into(),
            );
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
