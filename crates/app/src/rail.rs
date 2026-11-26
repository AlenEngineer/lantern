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
use iced::{Element, Fill, Length, Point};
use lantern_engine::{FlowOf, Plan, Repeat, StepId};

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
    /// The line of a drawn order was pressed beside a step: pulled along
    /// from there, it draws more of the order or rubs some of it out;
    /// let go where it was pressed, the steps it holds are picked.
    PressOrder(StepId, Option<lantern_engine::SpanId>),
    /// The same for the line of a loop: pulled along, that part of the
    /// loop is rubbed out.
    PressLoop(StepId, lantern_engine::SpanId),
    /// The pointer came onto a step's rail, or left it.
    Over(StepId),
    Left(StepId),
    /// A right click: the ring of what can be done to that thing.
    Ring(RingOn),
    RingClose,
    /// A section's ring: its colours fan out round its swatch, or fold.
    Colors,
    /// From the ring: what a key does to what is selected, done to the
    /// step or section the ring is open on.
    Do(StepId, crate::keys::Op),
    /// Takes a loop away: one drawn round some steps, or how often a
    /// step, a section or the plan comes.
    Unloop(FlowOf),
    /// Takes a drawn order away: of a span drawn round some steps, of a
    /// section, or of the plan.
    Unshuffle(FlowOf),
    /// How something that comes again until a time ends when it comes.
    Ends(FlowOf, lantern_engine::Ending),
}

/// Which way a pull has gone from the dot it began on, sideways: it
/// says which line is being drawn, as the pull goes, and can change as
/// it goes on. The lines are on the sides they are pulled to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// To the left of the dots: a loop, whichever way it runs.
    Left,
    /// Straight along the dots: up it draws a loop, down it joins.
    Along,
    /// To the right of the dots: a drawn order.
    Right,
}

/// How far to a side the pointer goes before the pull is to that side.
const ASIDE: f32 = 12.0;

/// What a ring is about. Each holds only what is that thing's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RingOn {
    /// A step, by its dot.
    Dot(StepId),
    /// The line from a step to the next: goes on, or waits.
    Line(StepId),
    /// A loop's line: how often it comes. Of a loop drawn round some
    /// steps, of a section, or of the plan.
    Loop(FlowOf),
    /// A drawn order's line: of a span, a section, or the plan.
    Shuffle(FlowOf),
    /// A section, by its header: what holds for all of it.
    All(StepId),
}

/// A ring that is open, and where: round the pointer as it was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    pub on: RingOn,
    pub at: Point,
    /// A section's colours are fanned out to pick from.
    pub colors: bool,
}

/// Where a section's swatch sits on its ring, out past the inner row,
/// how big it is, and how far round it its colours fan out.
const SWATCH_ANGLE: f32 = -69.0;
const SWATCH_FAR: f32 = 88.0;
const SWATCH: f32 = 26.0;
const FAN_FAR: f32 = 40.0;
const FAN: f32 = 22.0;

/// How big a ring's buttons are, and the ones of its outer row.
const PETAL: f32 = 34.0;
/// How far out the buttons on a ring's left are (what is done to it,
/// its note, its pages), 45° apart: far enough that two never touch.
const LEFT: f32 = 58.0;
/// How far a ring reaches out from its middle, its buttons and a
/// little room: the middle is kept this far in from the window's sides
/// and from the strip.
const REACH: f32 = LEFT + PETAL / 2.0 + 8.0;
/// How tall the pills are that say how often, and how far the card
/// round the time stands off its pill.
const PILL: f32 = 30.0;
const CARD: f32 = 6.0;
/// How wide the card is that says how a step fades.
/// How wide a section's card is under its ring.
const CARD_WIDE: f32 = 170.0;
const FADES: f32 = 300.0;

impl App {
    /// How often `of` comes, beside the ring's repeat button: a number
    /// of times with fewer and more, until you move on, or until a time
    /// of day typed there. With a time, under it: how it ends when the
    /// time comes, and what that comes to.
    fn how_often<'a>(
        &self,
        plan: &Plan,
        of: FlowOf,
        flow: lantern_engine::Flow,
    ) -> Element<'a, Message> {
        use lantern_engine::Ending;
        // The ring keeps its colours while all under it goes dim.
        let p = self.bright;
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
                        // Fewer than twice is once: no repeat at all.
                        match counted && times <= 2 {
                            true => Message::Rail(Act::Unloop(of)),
                            false => set(times.saturating_sub(1).max(2)),
                        }
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
        // The pills sit level, with the room a card round the time takes
        // over them: the time's pill grows into that card once a time is
        // set, with what belongs to the time inside it.
        let level = |pill: Element<'a, Message>| {
            container(pill).padding(iced::Padding::default().top(CARD))
        };
        let Some(time) = time else {
            return iced::widget::row![level(stepper), level(endless), level(until)]
                .spacing(8)
                .into();
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
        // The time's own card: the time on top, then how it ends, then
        // what that comes to.
        let card = container(
            iced::widget::column![until, ending, container(readout).padding([2, 8]),].spacing(8),
        )
        .padding(CARD)
        .style(move |_| container::Style {
            background: Some(p.surface_container.into()),
            border: iced::Border {
                radius: (PILL / 2.0 + CARD).into(),
                ..iced::Border::default()
            },
            shadow: iced::Shadow {
                color: iced::Color::BLACK.scale_alpha(0.45),
                offset: iced::Vector::new(0.0, 2.0),
                blur_radius: 10.0,
            },
            ..container::Style::default()
        });
        iced::widget::row![level(stepper), level(endless), card]
            .spacing(8)
            .into()
    }

    /// The ring that is open, as layers over the dashboard: something
    /// behind it to press to close it, then each of its buttons where it
    /// goes. Empty when none is open, or what it was about is gone.
    pub(crate) fn ring_layers(&self) -> Vec<Element<'_, Message>> {
        let Some(ring) = self.ring else {
            return Vec::new();
        };
        // The ring keeps its colours while all under it goes dim. A
        // section's is in the section's colour.
        let plan = self.engine.0.plan();
        let p = match ring.on {
            RingOn::All(id) => {
                let hue = crate::plan_pane::section_hue(self.bright, &plan.section_colors(), id);
                crate::palette::Palette {
                    primary: hue,
                    ..self.bright
                }
            }
            _ => self.bright,
        };
        // They fan out from the middle as it opens.
        let out = self.motion.lift();
        let close = Message::Rail(Act::RingClose);
        let mut layers: Vec<Element<'_, Message>> = vec![
            mouse_area(Space::new().width(Fill).height(Fill))
                .on_press(close.clone())
                .on_right_press(close)
                .into(),
        ];
        // Kept on the window and under the strip: near an edge the ring
        // moves in, all of its buttons in sight.
        let (wide, tall) = (self.dashboard_size.width, self.dashboard_size.height);
        let under_strip = crate::dashboard::EDGE + crate::dashboard::STRIP;
        // A section's colours fan out above it: further in from the top.
        let top = match ring.colors {
            true => under_strip + SWATCH_FAR + FAN_FAR + FAN / 2.0 + 8.0,
            false => under_strip + REACH,
        };
        let middle = Point::new(
            ring.at.x.clamp(REACH, (wide - REACH).max(REACH)),
            ring.at.y.clamp(top, (tall - 120.0).max(top)),
        );
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
        let mut again_at: Option<(FlowOf, lantern_engine::Flow, Point)> = None;
        match ring.on {
            RingOn::Dot(id) => {
                let Some(at) = plan.position(id) else {
                    return Vec::new();
                };
                let step = &plan.steps()[at];
                let goes = plan.goes_on(at);
                let again = step.flow.repeat != Repeat::Once;
                // Joined only to a step after it: the last has none.
                let joins = crate::plan_edit::goes_on_when(step).is_some()
                    && (goes || plan.next_step(at).is_some());
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
                    joins.then_some(Message::SetJoined(FlowOf::Step(id), !goes)),
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
                        FlowOf::Step(id),
                        if again {
                            Repeat::Once
                        } else {
                            Repeat::Times(2)
                        },
                    )),
                ));
                if again {
                    again_at = Some((FlowOf::Step(id), step.flow, place(30.0, 46.0)));
                }
                // How it fades, under the ring and clear of its buttons:
                // in and out, a second a press.
                if matches!(step.action, lantern_engine::Action::Show { .. }) {
                    let card = container(crate::plan_pane::fade_steppers(p, step, Length::Shrink))
                        .padding([6, 12])
                        .center_x(FADES)
                        .style(move |_| container::Style {
                            background: Some(p.surface_container.into()),
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
                        });
                    layers.push(
                        pin(card)
                            .x((middle.x - FADES / 2.0).max(8.0))
                            .y(middle.y + LEFT + PETAL / 2.0 + 4.0)
                            .into(),
                    );
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
                    Some(Message::SetJoined(FlowOf::Step(id), true)),
                ));
                petals.push(petal(
                    30.0,
                    44.0,
                    PETAL,
                    !goes,
                    glyph(Icon::Pause, !goes),
                    "Waits here",
                    Some(Message::SetJoined(FlowOf::Step(id), false)),
                ));
            }
            // A loop's line holds only how often it comes: there, by the
            // pointer, with no button of the ring to hang from.
            RingOn::Loop(of) => {
                let Some(flow) = plan.flow_of(of) else {
                    return Vec::new();
                };
                if flow.repeat == Repeat::Once {
                    return Vec::new();
                }
                layers.push(
                    pin(self.how_often(&plan, of, flow))
                        .x(middle.x + 14.0)
                        .y(middle.y - PILL / 2.0 - CARD)
                        .into(),
                );
            }
            // A drawn order's line holds one thing: that it is there.
            RingOn::Shuffle(of) => {
                if !plan.flow_of(of).is_some_and(|flow| flow.shuffle) {
                    return Vec::new();
                }
                petals.push(petal(
                    0.0,
                    34.0,
                    PETAL,
                    true,
                    glyph(Icon::Shuffle, true),
                    "In a drawn order. Press for the plan's order",
                    Some(Message::Rail(Act::Unshuffle(of))),
                ));
            }
            // A section: what holds for all of it, the span attached to
            // it. How often it comes again is its loop's line's.
            RingOn::All(of) => {
                let Some(flow) = plan.flow_of(of) else {
                    return Vec::new();
                };
                let of_flow = FlowOf::from(of);
                let again = flow.repeat != Repeat::Once;
                petals.push(petal(
                    -48.0,
                    50.0,
                    PETAL,
                    flow.joined,
                    glyph(Icon::Joined, flow.joined),
                    if flow.joined {
                        "Every step in it goes straight on. Press to take that away"
                    } else {
                        "Every step in it goes straight on"
                    },
                    Some(Message::SetJoined(of_flow, !flow.joined)),
                ));
                petals.push(petal(
                    0.0,
                    50.0,
                    PETAL,
                    again,
                    glyph(Icon::Repeat, again),
                    if again {
                        "All of it comes again. Press for once"
                    } else {
                        "All of it comes again"
                    },
                    Some(match again {
                        true => Message::Rail(Act::Unloop(of_flow)),
                        false => Message::SetRepeat(of_flow, Repeat::Times(2)),
                    }),
                ));
                petals.push(petal(
                    48.0,
                    50.0,
                    PETAL,
                    flow.shuffle,
                    glyph(Icon::Shuffle, flow.shuffle),
                    if flow.shuffle {
                        "All of it in a drawn order. Press for the plan's order"
                    } else {
                        "All of it in an order drawn afresh each time"
                    },
                    Some(match flow.shuffle {
                        true => Message::Rail(Act::Unshuffle(of_flow)),
                        false => Message::SetShuffle(of_flow, true),
                    }),
                ));
            }
        }
        // A section's ring has its card under it: its name, its times,
        // what it takes. Its fill is a button of the ring, while planning.
        let timing = self.timing();
        let card = match ring.on {
            RingOn::All(id) => plan.get(id).map(|step| {
                let section = timing.sections.iter().find(|section| section.id == id);
                self.section_card(p, &plan, step, section)
            }),
            _ => None,
        };
        // The card comes last, rising into place as the buttons settle.
        let rise = crate::motion::bubble(self.motion.ring_open(), 1, 2);
        if let Some(card) = card.filter(|_| rise > 0.05) {
            let card = container(card)
                .padding([8, 12])
                .width(CARD_WIDE)
                .style(move |_| container::Style {
                    background: Some(p.surface_container.into()),
                    border: iced::Border {
                        radius: 12.0.into(),
                        ..iced::Border::default()
                    },
                    shadow: iced::Shadow {
                        color: iced::Color::BLACK.scale_alpha(0.45),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    },
                    ..container::Style::default()
                });
            layers.push(
                pin(card)
                    .x((middle.x - CARD_WIDE / 2.0).max(8.0))
                    .y(middle.y + LEFT + PETAL / 2.0 + 6.0 + 16.0 * (1.0 - rise))
                    .into(),
            );
        }
        if let RingOn::All(id) = ring.on
            && self.mode == crate::mode::Mode::Plan
        {
            petals.push(petal(
                90.0,
                LEFT,
                PETAL,
                false,
                glyph(Icon::Fill, false),
                "Fill it from the library",
                Some(Message::Fill(crate::fill_panel::FillEdit::Open(id))),
            ));
        }
        // A deck or a PDF opens into its pages, while planning or in a
        // service, once its pages are known.
        if let RingOn::Dot(id) = ring.on
            && let Some(resource) = crate::plan_edit::paged(&plan, id)
        {
            let pages = self.media.pages(&resource.id).filter(|pages| *pages > 1);
            let words = match pages {
                Some(pages) => format!("Open it into its {pages} pages"),
                None => "Its pages are still being read".to_string(),
            };
            petals.push(petal(
                90.0,
                LEFT,
                PETAL,
                false,
                glyph(Icon::Document, false),
                &words,
                pages
                    .filter(|_| self.can_edit())
                    .map(|_| Message::OpenPages(id)),
            ));
        }
        // A section's colour: its swatch, out past the inner row. Pressed,
        // the colours fan out round it, and one pressed is the section's.
        if let RingOn::All(id) = ring.on {
            let slot = plan
                .section_colors()
                .into_iter()
                .find(|(section, _)| *section == id)
                .map_or(0, |(_, slot)| slot);
            let mut swatch = petal(
                SWATCH_ANGLE,
                SWATCH_FAR,
                SWATCH,
                ring.colors,
                Space::new().into(),
                "Its colour",
                Some(Message::Rail(Act::Colors)),
            );
            swatch.fill = Some(p.section(slot));
            petals.push(swatch);
            if ring.colors {
                let (sin, cos) = SWATCH_ANGLE.to_radians().sin_cos();
                let swatch = Point::new(SWATCH_FAR * cos, SWATCH_FAR * sin);
                for n in 0..lantern_engine::COLORS {
                    // Round the far side of the swatch, from the middle.
                    let turn = (SWATCH_ANGLE + 34.0 * (f32::from(n) - 2.5)).to_radians();
                    let at = Point::new(
                        swatch.x + FAN_FAR * turn.cos(),
                        swatch.y + FAN_FAR * turn.sin(),
                    );
                    let face: Element<'static, Message> = match n == slot {
                        true => look::icon(Icon::Check, 12.0, p.surface).into(),
                        false => Space::new().into(),
                    };
                    let mut color = petal(
                        at.y.atan2(at.x).to_degrees(),
                        at.x.hypot(at.y),
                        FAN,
                        false,
                        face,
                        "This colour",
                        Some(Message::SectionColor(id, n)),
                    );
                    color.fill = Some(p.section(n));
                    petals.push(color);
                }
            }
        }
        // A step's ring and a section's have, on the left, what is done
        // to it: renamed, copied, taken away. Its note is between.
        if let RingOn::Dot(id) | RingOn::All(id) = ring.on {
            let section = matches!(ring.on, RingOn::All(_));
            for (angle, icon, words, op) in [
                (
                    -90.0,
                    Icon::Pencil,
                    match section {
                        true => "Rename it",
                        false => "Rename it, set its length",
                    },
                    crate::keys::Op::Rename,
                ),
                (
                    -135.0,
                    Icon::Copy,
                    match section {
                        true => "A copy of all of it, after it",
                        false => "A copy of it, after it",
                    },
                    crate::keys::Op::Duplicate,
                ),
                (
                    135.0,
                    Icon::Close,
                    match section {
                        true => "Take the section away; its steps stay",
                        false => "Take it away",
                    },
                    crate::keys::Op::RemoveStep,
                ),
            ] {
                // A section is renamed in its card, under the ring, so
                // the ring stays.
                let does = match (section, op) {
                    (true, crate::keys::Op::Rename) => {
                        Message::EditStep(Some((id, crate::plan_edit::Part::Name)))
                    }
                    _ => Message::Rail(Act::Do(id, op)),
                };
                petals.push(petal(
                    angle,
                    LEFT,
                    PETAL,
                    false,
                    glyph(icon, false),
                    words,
                    Some(does),
                ));
            }
        }
        // Every ring has a note's place, on the left: what is noted of
        // what it is open on, opened, or a note for it made.
        let noted = match ring.on {
            RingOn::Dot(id) => Some(lantern_engine::NoteOn::Step(id)),
            RingOn::Loop(FlowOf::Span(id)) | RingOn::Shuffle(FlowOf::Span(id)) => {
                Some(lantern_engine::NoteOn::Span(id))
            }
            RingOn::Loop(FlowOf::Step(id))
            | RingOn::Shuffle(FlowOf::Step(id))
            | RingOn::All(id) => Some(lantern_engine::NoteOn::Step(id)),
            _ => None,
        };
        if let Some(on) = noted {
            let has = !plan.notes_on(on).is_empty();
            let mut note = petal(
                180.0,
                LEFT,
                PETAL,
                has,
                glyph(Icon::Note, has),
                if has { "Its note" } else { "A note on it" },
                Some(Message::NoteFor(vec![on])),
            );
            note.seen = Some(crate::track::Key::RingNote);
            petals.push(note);
        }
        if let Some((of, flow, at)) = again_at {
            let clear = (at.x + PETAL / 2.0 + 8.0).max(middle.x + 50.0 + PETAL / 2.0 + 8.0);
            layers.push(
                // Clear of the ring's buttons, level with the repeat one.
                pin(self.how_often(&plan, of, flow))
                    .x(clear)
                    .y(at.y - PILL / 2.0 - CARD)
                    .into(),
            );
            // A thin line from the repeat button to its choices: they
            // are its own.
            let from = at.x + PETAL / 2.0;
            layers.push(
                pin(
                    container(Space::new().width((clear - from).max(0.0)).height(1.5)).style(
                        move |_| {
                            container::Style::default()
                                .background(p.primary.scale_alpha(0.55 * out))
                        },
                    ),
                )
                .x(from)
                .y(at.y - 0.75)
                .into(),
            );
        }
        // They pop out one after another, like bubbles.
        let open = self.motion.ring_open();
        let count = petals.len();
        for (n, petal) in petals.into_iter().enumerate() {
            let pop = crate::motion::bubble(open, n, count);
            let Petal {
                angle,
                far,
                size,
                lit,
                face,
                about,
                message,
                fill,
                seen,
            } = petal;
            let (sin, cos) = angle.to_radians().sin_cos();
            let far = far * out * pop.max(0.0);
            // It grows as it comes: a bubble, not a button sliding out.
            let size = size * (0.4 + 0.6 * pop).max(0.0);
            let shown = pop.clamp(0.0, 1.0);
            let ground = match (fill, lit) {
                (Some(fill), _) => fill,
                (None, true) => p.primary,
                (None, false) => {
                    crate::motion::blend(p.surface_container_highest, p.on_surface, 0.1)
                }
            };
            // A colour's own button is ringed while its choices are open.
            let edge = match (fill, lit) {
                (Some(_), true) => iced::Border {
                    color: p.on_surface.scale_alpha(out * shown),
                    width: 2.0,
                    radius: (size / 2.0).into(),
                },
                _ => iced::Border {
                    radius: (size / 2.0).into(),
                    ..iced::Border::default()
                },
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
                        .scale_alpha(out * shown)
                        .into(),
                    ),
                    border: edge,
                    shadow: iced::Shadow {
                        color: iced::Color::BLACK.scale_alpha(0.45 * out * shown),
                        offset: iced::Vector::new(0.0, 2.0),
                        blur_radius: 8.0,
                    },
                    ..button::Style::default()
                });
            let round: Element<'_, Message> = match seen {
                Some(key) => crate::track::seen(key, &self.places, round),
                None => round.into(),
            };
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
    /// Its ground, where it is a colour.
    fill: Option<iced::Color>,
    /// The key it says its place under, for a script's pointer.
    seen: Option<crate::track::Key>,
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
        fill: None,
        seen: None,
    }
}

/// What letting go would do, to the rows from `first` to `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Would {
    /// A loop round them.
    Loop { first: usize, last: usize },
    /// Each goes on into the next.
    Join { first: usize, last: usize },
    /// They come in a drawn order.
    Shuffle { first: usize, last: usize },
    /// They leave the drawn order they are in: it is rubbed out there.
    Rub { first: usize, last: usize },
    /// Nothing: no loop or drawn order can be there, and why not.
    Not {
        first: usize,
        last: usize,
        why: lantern_engine::SpanError,
    },
}

/// Why a pull to `side` is refused, in a few words.
pub fn why_not(why: lantern_engine::SpanError, side: Side) -> &'static str {
    use lantern_engine::SpanError;
    match (why, side) {
        (SpanError::TooFew, Side::Right) => "A drawn order takes three steps or more",
        (SpanError::TooFew, _) => "A loop takes two steps or more",
        (SpanError::AcrossHeading, _) => "A drawn order can't cross a heading",
        (SpanError::HalfOver(_), Side::Right) => "It would cut another drawn order or loop in two",
        (SpanError::HalfOver(_), _) => "It would cut another loop or drawn order in two",
        (SpanError::Missing, _) => "That is no longer in the plan",
    }
}

impl Would {
    /// The rows it is about.
    pub fn rows(self) -> (usize, usize) {
        match self {
            Would::Loop { first, last }
            | Would::Join { first, last }
            | Would::Shuffle { first, last }
            | Would::Rub { first, last }
            | Would::Not { first, last, .. } => (first, last),
        }
    }
}

/// A dot held down.
#[derive(Debug, Clone, PartialEq)]
pub struct Pull {
    pub from: StepId,
    /// Which way it has gone, sideways, from where it began.
    pub side: Side,
    /// The side it keeps whichever way it goes: one begun on a line, not
    /// on a dot, is about that line.
    held: Option<Side>,
    /// The loop or drawn order whose line it began on.
    pub line: Option<lantern_engine::SpanId>,
    /// It began on a loop's line: what it goes over is rubbed out of
    /// that loop.
    pub rubs: bool,
    /// The step it was last over: it stays the one, past the rail's
    /// edge where there is no row under the pointer.
    over: Option<StepId>,
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
            side: Side::Along,
            held: None,
            line: None,
            rubs: false,
            over: None,
            toggle,
            origin: None,
            at: Point::ORIGIN,
            moving: false,
            would: None,
        }
    }

    /// A press on the line of a drawn order beside `from`: a pull from
    /// there is to that side, wherever it goes.
    pub fn on_order(from: StepId, line: Option<lantern_engine::SpanId>) -> Pull {
        Pull {
            side: Side::Right,
            held: Some(Side::Right),
            line,
            ..Pull::new(from, None)
        }
    }

    /// A press on the line of the loop `line` beside `from`: a pull from
    /// there rubs what it goes over out of that loop.
    pub fn on_loop(from: StepId, line: lantern_engine::SpanId) -> Pull {
        Pull {
            side: Side::Left,
            held: Some(Side::Left),
            line: Some(line),
            rubs: true,
            ..Pull::new(from, None)
        }
    }

    /// The pointer moved to `at`.
    pub fn to(&mut self, at: Point) {
        let origin = *self.origin.get_or_insert(at);
        self.at = at;
        if origin.distance(at) > SLOP {
            self.moving = true;
        }
        self.side = self.held.unwrap_or(match at.x - origin.x {
            aside if aside < -ASIDE => Side::Left,
            aside if aside > ASIDE => Side::Right,
            _ => Side::Along,
        });
    }

    /// Works out what letting go over `over` would do. To the left a
    /// loop, to the right a drawn order, whichever way it runs; straight
    /// along the dots, a loop back up or a join on down.
    pub fn aim(&mut self, plan: &Plan, over: Option<StepId>) {
        self.over = over.or(self.over);
        self.would = self
            .moving
            .then_some(self.over)
            .flatten()
            .filter(|over| *over != self.from)
            .and_then(|over| {
                let (from, to) = (plan.position(self.from)?, plan.position(over)?);
                if plan.steps()[to].is_section() {
                    return None;
                }
                let (first, last) = (from.min(to), from.max(to));
                let or_not = |can: Result<_, lantern_engine::SpanError>, would: Would| match can {
                    Ok(_) => would,
                    Err(why) => Would::Not { first, last, why },
                };
                Some(match self.side {
                    // An order drawn by hand: new, joined to one it
                    // meets, or rubbed out of one it is inside.
                    Side::Right => match plan.would_draw_over(self.from, over) {
                        Ok(lantern_engine::Drawn::Rubbed) => Would::Rub { first, last },
                        Ok(_) => Would::Shuffle { first, last },
                        Err(why) => Would::Not { first, last, why },
                    },
                    Side::Along if to > from => Would::Join { first, last },
                    // Begun on a loop's line: out of that loop.
                    Side::Left if self.rubs => match self.line {
                        Some(line) => or_not(
                            plan.would_rub_loop(line, self.from, over),
                            Would::Rub { first, last },
                        ),
                        None => return None,
                    },
                    // A loop drawn by hand: new, or one with a loop it
                    // takes in a part of.
                    Side::Left | Side::Along => or_not(
                        plan.would_loop_by_hand(self.from, over).map(|_| ()),
                        Would::Loop { first, last },
                    ),
                })
            });
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::Action;

    use super::*;

    #[test]
    fn the_way_a_pull_goes_says_which_line_it_draws() {
        let mut plan = Plan::new("Sabbath");
        plan.section("Music", None);
        let [a, b, c] = ["A", "B", "C"].map(|name| plan.push(name, Action::Blank));
        plan.section("Sermon", None);
        let d = plan.push("D", Action::Blank);
        let x = 100.0;
        let pull = |from, to: (f32, f32), over| {
            let mut pull = Pull::new(from, Some(true));
            pull.to(Point::new(x, 100.0));
            pull.aim(&plan, over);
            assert_eq!(pull.would, None, "not moved yet: a click");
            pull.to(Point::new(to.0, to.1));
            pull.aim(&plan, over);
            pull
        };
        // Straight up from C to A: a loop. Back on itself, nothing.
        let mut up = pull(c, (x, 40.0), Some(a));
        assert_eq!(up.would, Some(Would::Loop { first: 1, last: 3 }));
        up.aim(&plan, Some(c));
        assert_eq!(up.would, None);
        // Straight down from A to C: A and B go on.
        assert_eq!(
            pull(a, (x, 160.0), Some(c)).would,
            Some(Would::Join { first: 1, last: 3 })
        );
        // Down and to the left: a loop all the same.
        assert_eq!(
            pull(a, (x - 20.0, 160.0), Some(c)).would,
            Some(Would::Loop { first: 1, last: 3 })
        );
        // To the right, up or down: a drawn order; over two, none.
        assert_eq!(
            pull(a, (x + 20.0, 160.0), Some(c)).would,
            Some(Would::Shuffle { first: 1, last: 3 })
        );
        assert_eq!(
            pull(c, (x + 20.0, 40.0), Some(a)).would,
            Some(Would::Shuffle { first: 1, last: 3 })
        );
        assert_eq!(
            pull(a, (x + 20.0, 130.0), Some(b)).would,
            Some(Would::Not {
                first: 1,
                last: 2,
                why: lantern_engine::SpanError::TooFew
            })
        );
        // It changes as it goes: back to the middle, down is a join again.
        let mut wandering = pull(a, (x + 20.0, 160.0), Some(c));
        wandering.to(Point::new(x + 2.0, 160.0));
        wandering.aim(&plan, Some(c));
        assert_eq!(wandering.would, Some(Would::Join { first: 1, last: 3 }));
        // Past the rail's edge, with no row under the pointer, it is
        // still over the last one it was.
        wandering.to(Point::new(x - 40.0, 160.0));
        wandering.aim(&plan, None);
        assert_eq!(wandering.would, Some(Would::Loop { first: 1, last: 3 }));
        // From D up to B, across Sermon's heading: a loop goes there, a
        // drawn order does not.
        assert_eq!(
            pull(d, (x, 60.0), Some(b)).would,
            Some(Would::Loop { first: 2, last: 5 })
        );
        plan.push("E", Action::Blank);
        let pull = |from, to: (f32, f32), over| {
            let mut pull = Pull::new(from, Some(true));
            pull.to(Point::new(x, 100.0));
            pull.to(Point::new(to.0, to.1));
            pull.aim(&plan, over);
            pull
        };
        assert_eq!(
            pull(d, (x + 20.0, 60.0), Some(a)).would,
            Some(Would::Not {
                first: 1,
                last: 5,
                why: lantern_engine::SpanError::AcrossHeading
            })
        );
    }
}
