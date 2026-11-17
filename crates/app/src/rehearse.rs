//! Rehearsing: a section, or the whole plan, walked through in the
//! dashboard's preview as the service would take it, with nothing on the
//! projector.
//!
//! It follows the flow: a step that goes on stays for its length and
//! then the next comes; one that waits stays until it is moved on, as
//! it would in the service; what repeats comes round, and a shuffled
//! section comes in a drawn order. A video is a still for its length. A
//! speed runs the clock faster, so a long plan can be seen through in a
//! few minutes.

use std::time::{Duration, Instant};

use iced::widget::{Space, button, column, container, row, text, tooltip};
use iced::{Alignment, Element, Fill, Task};
use lantern_engine::{Session, Step, StepId};

use crate::dashboard::hinted;
use crate::look::{self, BODY, BOLD, Icon, META, Tone};
use crate::{App, Message, length, plan_pane};

/// The speeds, in the order the button goes through them.
const SPEEDS: [u32; 3] = [1, 4, 16];

/// A step is looked at this long at least, however short it is.
const LEAST: Duration = Duration::from_secs(1);

/// A rehearsal under way.
pub struct Rehearsal {
    /// What is rehearsed: a section, or the plan (`None`).
    pub of: Option<StepId>,
    /// The part being walked, as a plan of its own.
    session: Session,
    /// How long the current step has been up, on the rehearsal's clock,
    /// up to when it last stopped or changed speed.
    before: Duration,
    /// Since when the clock has run; `None` while paused.
    since: Option<Instant>,
    /// How many times faster than the service its clock runs.
    pub speed: u32,
    /// How long the steps before this one took, on its clock.
    took: Duration,
}

impl Rehearsal {
    /// How long the current step has been up, on the rehearsal's clock.
    fn up(&self) -> Duration {
        self.before
            + self
                .since
                .map_or(Duration::ZERO, |since| since.elapsed() * self.speed)
    }

    fn paused(&self) -> bool {
        self.since.is_none()
    }

    /// Stops or starts the clock where it is.
    fn hold(&mut self, hold: bool) {
        self.before = self.up();
        self.since = (!hold).then(Instant::now);
    }

    pub fn current(&self) -> Option<&Step> {
        self.session.current()
    }

    /// Whether the current step goes on by itself.
    fn goes_on(&self) -> bool {
        self.session
            .current_index()
            .is_some_and(|at| self.session.plan().goes_on(at))
    }
}

impl App {
    /// How long `step` stays up in a rehearsal.
    fn rehearsed_length(&self, step: &Step) -> Duration {
        lantern_engine::timing::estimate(step, self.loaded_length(step))
            .length
            .max(LEAST)
    }

    /// Starts rehearsing `of`, or stops if it is what is being rehearsed.
    pub(crate) fn rehearse(&mut self, of: Option<StepId>) -> Task<Message> {
        self.rehearsed = None;
        if self.rehearsal.take().is_some_and(|was| was.of == of) {
            return Task::none();
        }
        let plan = self.engine.0.plan();
        let Some(part) = (match of {
            Some(id) => plan.part(id),
            None => Some(plan),
        }) else {
            return Task::none();
        };
        let mut session = Session::new(part);
        session.tell_time(length::now_of_day());
        let Some(first) = session.next().map(|step| step.id) else {
            return Task::none();
        };
        crate::log("rehearsal: starts");
        self.rehearsal = Some(Rehearsal {
            of,
            session,
            before: Duration::ZERO,
            since: Some(Instant::now()),
            speed: SPEEDS[0],
            took: Duration::ZERO,
        });
        self.update(Message::Select(first))
    }

    /// On to the next step now. After the last, the rehearsal is over.
    pub(crate) fn rehearse_next(&mut self) -> Task<Message> {
        let Some(rehearsal) = &mut self.rehearsal else {
            return Task::none();
        };
        rehearsal.took += rehearsal.up();
        rehearsal.session.tell_time(length::now_of_day());
        match rehearsal.session.next().map(|step| step.id) {
            Some(next) => {
                rehearsal.before = Duration::ZERO;
                rehearsal.since = Some(Instant::now());
                self.update(Message::Select(next))
            }
            None => {
                crate::log("rehearsal: over");
                self.rehearsed = Some((rehearsal.of, rehearsal.took));
                self.rehearsal = None;
                Task::none()
            }
        }
    }

    /// The clock's tick: a step that goes on, its time up, goes on.
    pub(crate) fn rehearse_tick(&mut self) -> Task<Message> {
        let due = self.rehearsal.as_ref().is_some_and(|rehearsal| {
            !rehearsal.paused()
                && rehearsal.goes_on()
                && rehearsal
                    .current()
                    .is_some_and(|step| rehearsal.up() >= self.rehearsed_length(step))
        });
        match due {
            true => self.rehearse_next(),
            false => Task::none(),
        }
    }

    pub(crate) fn rehearse_pause(&mut self) {
        if let Some(rehearsal) = &mut self.rehearsal {
            let paused = rehearsal.paused();
            rehearsal.hold(!paused);
        }
    }

    /// The next speed, round again after the fastest.
    pub(crate) fn rehearse_speed(&mut self) {
        if let Some(rehearsal) = &mut self.rehearsal {
            // The time so far is kept at the speed it ran at.
            let paused = rehearsal.paused();
            rehearsal.hold(paused);
            let at = SPEEDS.iter().position(|speed| *speed == rehearsal.speed);
            rehearsal.speed = SPEEDS[at.map_or(0, |at| (at + 1) % SPEEDS.len())];
        }
    }

    /// How far through `step` the rehearsal is, when it is the one up.
    pub(crate) fn rehearsed_part(&self, step: StepId) -> Option<f32> {
        let rehearsal = self.rehearsal.as_ref()?;
        let current = rehearsal.current().filter(|current| current.id == step)?;
        let length = self.rehearsed_length(current);
        Some((rehearsal.up().as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0))
    }

    /// Under the preview: the rehearsal's buttons, how far the step is,
    /// and what comes after it; or, just over, how long it took.
    pub(crate) fn rehearsal_controls(&self) -> Option<Element<'_, Message>> {
        let p = self.palette;
        let round = |content: Element<'static, Message>, tone, about: &str, message| {
            hinted(
                p,
                button(container(content).center_x(32).center_y(32))
                    .padding(0)
                    .on_press(message)
                    .style(look::pill(p, tone)),
                about.to_string(),
                tooltip::Position::Top,
            )
        };
        let Some(rehearsal) = &self.rehearsal else {
            // Over: how long it really took, and the way to run it again.
            let (of, took) = self.rehearsed?;
            let plan = self.engine.0.plan();
            let name = match of {
                Some(id) => plan.get(id)?.label.clone(),
                None => plan.title.clone(),
            };
            return Some(
                row![
                    round(
                        look::icon(Icon::Play, 16.0, p.on_surface).into(),
                        Tone::Plain,
                        "Rehearse it again",
                        Message::Rehearse(of),
                    ),
                    look::icon(Icon::Check, 16.0, p.primary),
                    container(text(name).size(BODY).wrapping(text::Wrapping::None))
                        .width(Fill)
                        .clip(true),
                    text(length::clock(took))
                        .size(META)
                        .color(p.on_surface_variant),
                ]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            );
        };
        let step = rehearsal.current()?;
        let length = self.rehearsed_length(step);
        let up = rehearsal.up().min(length);
        let goes = rehearsal.goes_on();
        // Its time up and it waits: the way on is the lit one.
        let waiting = !goes && rehearsal.up() >= length;
        let ink = |lit: bool| if lit { p.on_primary } else { p.on_surface };
        let tone = |lit: bool| if lit { Tone::Solid } else { Tone::Plain };
        let playing = !rehearsal.paused() && !waiting;
        let controls = row![
            round(
                look::icon(
                    if rehearsal.paused() {
                        Icon::Play
                    } else {
                        Icon::Pause
                    },
                    16.0,
                    ink(playing)
                )
                .into(),
                tone(playing),
                if rehearsal.paused() {
                    "Go on rehearsing"
                } else {
                    "Pause"
                },
                Message::RehearsePause,
            ),
            round(
                look::icon(Icon::SkipNext, 16.0, ink(waiting)).into(),
                tone(waiting),
                "On to the next step now",
                Message::RehearseNext,
            ),
            container(plan_pane::bar(
                p,
                up.as_secs_f32() / length.as_secs_f32(),
                p.primary,
                0.0
            ))
            .width(Fill),
            text(format!("{} / {}", length::clock(up), length::clock(length)))
                .size(META)
                .color(p.on_surface_variant),
            hinted(
                p,
                button(
                    text(format!("{}×", rehearsal.speed))
                        .size(META)
                        .font(BOLD)
                        .color(p.primary)
                )
                .padding([4, 9])
                .on_press(Message::RehearseSpeed)
                .style(look::pill(p, Tone::Plain)),
                "How fast its clock runs".to_string(),
                tooltip::Position::Top,
            ),
            round(
                look::icon(Icon::Stop, 16.0, p.on_surface).into(),
                Tone::Plain,
                "Stop rehearsing",
                Message::Rehearse(rehearsal.of),
            ),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        // What comes after it, and whether by itself.
        let after: Element<'_, Message> = match rehearsal.session.peek_next() {
            Some(next) => row![
                look::icon(
                    if goes { Icon::Joined } else { Icon::Pause },
                    13.0,
                    p.primary
                ),
                container(
                    text(next.label.clone())
                        .size(META)
                        .color(p.on_surface_variant)
                        .wrapping(text::Wrapping::None)
                )
                .width(Fill)
                .clip(true),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
            None => Space::new().into(),
        };
        Some(column![controls, after].spacing(8).into())
    }
}
