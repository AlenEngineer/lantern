//! The dashboard's motion: what moves for a moment to say that something
//! happened, and then stops.
//!
//! Every one is short and plays once. Nothing loops: the booth is dark, and
//! a screen that keeps pulsing pulls the eye from the stage. None delays
//! what it follows either; the projector has changed before its ring is
//! drawn.
//!
//! Each is one of iced's animations, set going at the moment it started.
//! The view asks how far along it is, and the app asks for frames only
//! while one is running.

use std::time::{Duration, Instant};

use iced::animation::{Animation, Easing};
use lantern_engine::{FlowOf, ResourceId, SceneId, StepId};

use crate::mode::Mode;

/// The ring around a step's dot as it goes on the projector.
pub const RING: Duration = Duration::from_millis(700);
/// How long the stage says the phone moved it.
pub const PHONE: Duration = Duration::from_secs(2);
/// The nudge of the mode's note when something is refused.
pub const NUDGE: Duration = Duration::from_millis(240);
/// How long the note says why it was refused.
pub const WHY: Duration = Duration::from_millis(1800);
/// The flash of a step that was just added, replaced or moved.
pub const FLASH: Duration = Duration::from_millis(600);
/// The lit pill of the mode switch sliding to the mode.
pub const SLIDE: Duration = Duration::from_millis(220);
/// The gap opening where what is carried would land.
pub const GAP: Duration = Duration::from_millis(150);
/// The gap going in or out as what is carried changes level.
pub const LEVEL: Duration = Duration::from_millis(140);
/// The card of what was just picked up, rising under the pointer.
pub const LIFT: Duration = Duration::from_millis(160);
/// How long a ring takes to open, all its buttons popped out.
pub const BUBBLE: Duration = Duration::from_millis(420);

/// How far out the `n`th of `of` buttons of a ring is when it has
/// opened `t` of the way: each pops out a little after the one before,
/// past where it goes and back, like a bubble. 0 before it starts, 1
/// once it has settled.
pub fn bubble(t: f32, n: usize, of: usize) -> f32 {
    // Each takes this much of the time; the rest is how late the last
    // one starts.
    const POP: f32 = 0.6;
    let late = match of {
        0 | 1 => 0.0,
        of => (1.0 - POP) * n as f32 / (of - 1) as f32,
    };
    let x = ((t - late) / POP).clamp(0.0, 1.0);
    // An overshoot that settles: easing out "back".
    const BACK: f32 = 1.9;
    let x = x - 1.0;
    1.0 + (BACK + 1.0) * x * x * x + BACK * x * x
}
/// A search result travelling to its row in the plan.
pub const FLIGHT: Duration = Duration::from_millis(420);
/// Each thing a fill proposes dropping in, one after another.
pub const DROP: Duration = Duration::from_millis(120);
/// All a fill proposes dropping in, at most.
pub const DROPS: Duration = Duration::from_millis(600);
/// What a fill put in, its dashed outline fading to a plain row.
pub const SOLID: Duration = Duration::from_millis(500);
/// A section put over its time: its sum turning red, its bracket bulging.
pub const TIPPED: Duration = Duration::from_millis(600);
/// A light running back along a loop's line as the service goes round.
pub const ROUND: Duration = Duration::from_millis(700);
/// Rows taken away closing up, or put back growing in.
pub const ROWS: Duration = Duration::from_millis(220);
/// A service going on: the ring of what went on opening, what was on
/// fading to played.
pub const STEP: Duration = Duration::from_millis(500);
/// Save, a tick for a moment after it is pressed.
pub const SAVED: Duration = Duration::from_millis(1200);
/// An empty plan's + breathing, all its breaths.
pub const BREATH: Duration = Duration::from_millis(3600);
const BREATHS: f32 = 3.0;
/// The phone's press and what it put on stage are this close together.
const SOON: Duration = Duration::from_millis(1500);

/// Something that moves once: when it started, and the animation set
/// going then.
#[derive(Debug, Clone)]
struct Run {
    since: Instant,
    going: Animation<bool>,
}

impl Run {
    /// Starts at `since` and takes `length`, along `easing`.
    fn new(since: Instant, length: Duration, easing: Easing) -> Run {
        Run {
            since,
            going: Animation::new(false)
                .duration(length)
                .easing(easing)
                .go(true, since),
        }
    }

    fn running(&self, now: Instant) -> bool {
        self.going.is_animating(now)
    }

    /// How far along it is at `now`, from 0 at its start: `None` once
    /// it is over.
    fn along(&self, now: Instant) -> Option<f32> {
        self.running(now)
            .then(|| self.going.interpolate(0.0, 1.0, now))
    }
}

pub struct Motion {
    /// Nothing moves: every change is drawn as it ends up.
    pub still: bool,
    /// Every length is this many times longer, to look at one closely.
    slow: f32,
    /// What is on the projector, and the ring of a new thing going up.
    program: Option<SceneId>,
    went_up: Option<Run>,
    /// When the phone last did something to the stage.
    phone: Option<Instant>,
    /// What was last refused, in the mode's words: its light fading, and
    /// the shake of the head.
    refused: Option<(&'static str, Run, Run)>,
    /// The step that last changed, and its flash.
    changed: Option<(StepId, Run)>,
    /// The mode it is in, the one it was in before, and the slide between.
    mode: Mode,
    left: Option<(Mode, Run)>,
    /// Where what is carried would land, opening.
    gap: Option<(usize, Run)>,
    /// Where it would have landed before that, closing.
    closing: Option<(usize, Run)>,
    /// How far in the gap is, in pixels: from where, to where.
    indent: Option<(f32, f32, Run)>,
    /// What is carried was just picked up.
    picked: Option<Run>,
    /// A ring opening: its buttons pop out one after another.
    ringing: Option<Run>,
    /// The section what is carried would be a part of, lighting up.
    host: Option<(StepId, Run)>,
    /// The plan was just saved: its Save is a tick for a moment.
    saved: Option<Run>,
    /// The plan just became empty: its + breathes a few times, then
    /// rests.
    breath: Option<Run>,
    /// Rows just taken away, closing up before the row named (the end of
    /// the plan for none): how many.
    gone: Option<(Option<StepId>, usize, Run)>,
    /// Rows just put back, growing in.
    came: Option<(Vec<StepId>, Run)>,
    /// What the service just went round, its light running back.
    round: Option<(FlowOf, Run)>,
    /// The service just went on, from the step named.
    stepped: Option<(Option<StepId>, Run)>,
    /// Sections an edit just put over their time.
    tipped: Option<(Vec<StepId>, Run)>,
    /// What a fill just proposed, dropping in one after another.
    proposed: Option<(Vec<ResourceId>, Run)>,
    /// Steps a fill just put in, their dashed outlines fading.
    accepted: Option<(Vec<StepId>, Run)>,
    /// A search result just put in the plan, travelling from where it
    /// was found to its row: the step, its title, and where it set off.
    flight: Option<(StepId, String, iced::Point, Run)>,
}

impl Motion {
    pub fn new() -> Motion {
        let slow = std::env::var("LANTERN_MOTION_SLOW")
            .ok()
            .and_then(|times| times.parse::<f32>().ok())
            .filter(|times| times.is_finite() && *times > 0.0)
            .unwrap_or(1.0);
        Motion {
            still: false,
            slow,
            program: None,
            went_up: None,
            phone: None,
            refused: None,
            changed: None,
            mode: Mode::Plan,
            left: None,
            gap: None,
            closing: None,
            indent: None,
            picked: None,
            ringing: None,
            host: None,
            saved: None,
            breath: None,
            gone: None,
            came: None,
            round: None,
            stepped: None,
            tipped: None,
            proposed: None,
            accepted: None,
            flight: None,
        }
    }

    fn length(&self, of: Duration) -> Duration {
        of.mul_f32(self.slow)
    }

    /// Something that takes `length`, started at `since`: fast at first,
    /// settling at the end.
    fn run(&self, length: Duration, since: Instant) -> Run {
        Run::new(since, self.length(length), Easing::EaseOutCubic)
    }

    /// How far along `run` is: 0 at its start, and `None` once it is
    /// over, or with motion off.
    fn along(&self, run: Option<&Run>) -> Option<f32> {
        if self.still {
            return None;
        }
        run?.along(Instant::now())
    }

    /// The plan was saved just now.
    pub fn saved(&mut self) {
        self.saved = Some(Run::new(Instant::now(), self.length(SAVED), Easing::Linear));
    }

    /// How much the tick that took Save's place shows: whole, then fading
    /// over the last part.
    pub fn tick(&self) -> Option<f32> {
        let t = self.along(self.saved.as_ref())?;
        Some((1.0 - (t - 0.6).max(0.0) / 0.4).clamp(0.0, 1.0))
    }

    /// A search result titled `title` went into the plan as `step`, from
    /// `from`.
    pub fn flies(&mut self, step: StepId, title: String, from: iced::Point) {
        self.flight = Some((step, title, from, self.run(FLIGHT, Instant::now())));
        self.rows_came(vec![step]);
    }

    /// The result on its way to its row: which step, its title, where it
    /// set off, and how far along it is.
    pub fn flight(&self) -> Option<(StepId, &str, iced::Point, f32)> {
        let (step, title, from, run) = self.flight.as_ref()?;
        Some((*step, title.as_str(), *from, self.along(Some(run))?))
    }

    /// A fill just proposed `new`.
    pub fn proposed(&mut self, new: Vec<ResourceId>) {
        // However many, all are in within a moment.
        let length = (DROP * new.len() as u32).min(DROPS);
        self.proposed = Some((
            new,
            Run::new(Instant::now(), self.length(length), Easing::Linear),
        ));
    }

    /// How far a proposal has dropped in, 0 to 1: each starts after the
    /// one before it. `None` once all are in, or for one not new.
    pub fn dropping(&self, id: &ResourceId) -> Option<f32> {
        let (new, run) = self.proposed.as_ref()?;
        let at = new.iter().position(|one| one == id)?;
        let t = self.along(Some(run))? * new.len() as f32;
        let own = (t - at as f32).clamp(0.0, 1.0);
        Some(1.0 - (1.0 - own).powi(3))
    }

    /// A fill just put `steps` into the plan.
    pub fn accepted(&mut self, steps: Vec<StepId>) {
        self.accepted = Some((steps, self.run(SOLID, Instant::now())));
    }

    /// How far a step a fill just put in has gone from dashed to solid.
    pub fn solidifying(&self, step: StepId) -> Option<f32> {
        let (steps, run) = self.accepted.as_ref()?;
        steps.contains(&step).then_some(())?;
        self.along(Some(run))
    }

    /// An edit just put `sections` over their time.
    pub fn tipped_over(&mut self, sections: Vec<StepId>) {
        self.tipped = Some((sections, self.run(TIPPED, Instant::now())));
    }

    /// How far a section just put over its time has gone: its sum turning
    /// red, its bracket bulging and settling.
    pub fn tipped(&self, section: StepId) -> Option<f32> {
        let (sections, run) = self.tipped.as_ref()?;
        sections.contains(&section).then_some(())?;
        self.along(Some(run))
    }

    /// The service went round `of`.
    pub fn went_round(&mut self, of: FlowOf) {
        self.round = Some((of, self.run(ROUND, Instant::now())));
    }

    /// How far the light has run back along `of`'s line.
    pub fn round_pulse(&self, of: FlowOf) -> Option<f32> {
        self.going_round()
            .filter(|(on, _)| *on == of)
            .map(|(_, t)| t)
    }

    /// What the service is going round, and how far the light has run.
    pub fn going_round(&self) -> Option<(FlowOf, f32)> {
        let (on, run) = self.round.as_ref()?;
        Some((*on, self.along(Some(run))?))
    }

    /// The service went on from `off`.
    pub fn went_on(&mut self, off: Option<StepId>) {
        self.stepped = Some((off, self.run(STEP, Instant::now())));
    }

    /// How far the ring of what just went on has opened.
    pub fn stepped(&self) -> Option<f32> {
        self.along(self.stepped.as_ref().map(|(_, run)| run))
    }

    /// How far `step`, just gone off, has faded to played.
    pub fn going_off(&self, step: StepId) -> Option<f32> {
        let (off, run) = self.stepped.as_ref()?;
        (*off == Some(step)).then_some(())?;
        self.along(Some(run))
    }

    /// `rows` rows were taken away from before `anchor`.
    pub fn rows_gone(&mut self, anchor: Option<StepId>, rows: usize) {
        self.gone = Some((anchor, rows, self.run(ROWS, Instant::now())));
    }

    /// How much room the rows taken away still take before `anchor`, as a
    /// share of their height: 1 just taken, 0 closed.
    pub fn gone_before(&self, anchor: Option<StepId>) -> Option<(usize, f32)> {
        let (at, rows, run) = self.gone.as_ref()?;
        (*at == anchor).then_some(())?;
        Some((*rows, 1.0 - self.along(Some(run))?))
    }

    /// The rows `came` were put back.
    pub fn rows_came(&mut self, came: Vec<StepId>) {
        self.came = Some((came, self.run(ROWS, Instant::now())));
    }

    /// How far a row put back has grown in: `None` once it is whole.
    pub fn growing(&self, step: StepId) -> Option<f32> {
        let (came, run) = self.came.as_ref()?;
        came.contains(&step).then_some(())?;
        self.along(Some(run))
    }

    /// The plan is empty now, or not: becoming empty, its + breathes.
    pub fn empty_plan(&mut self, empty: bool) {
        match (empty, self.breath.is_some()) {
            (true, false) => {
                self.breath = Some(Run::new(
                    Instant::now(),
                    self.length(BREATH),
                    Easing::Linear,
                ));
            }
            (false, true) => self.breath = None,
            _ => {}
        }
    }

    /// How far into a breath the + is: 0 at rest, 1 at its fullest.
    pub fn breath(&self) -> f32 {
        self.along(self.breath.as_ref()).map_or(0.0, |t| {
            (1.0 - (t * BREATHS * std::f32::consts::TAU).cos()) / 2.0
        })
    }

    /// The projector shows `program` now. Something new going up rings.
    pub fn stage_shows(&mut self, program: Option<SceneId>) {
        if program != self.program {
            self.program = program;
            self.went_up = program.map(|_| self.run(RING, Instant::now()));
        }
    }

    /// The ring of what just went on the projector.
    pub fn ring(&self) -> Option<f32> {
        self.along(self.went_up.as_ref())
    }

    /// The phone did something to the stage.
    pub fn phone_acted(&mut self) {
        self.phone = Some(Instant::now());
    }

    /// The stage was last moved from the phone, a moment ago. Said with
    /// motion off too: it is something to know, not something moving.
    pub fn by_phone(&self) -> bool {
        self.phone
            .is_some_and(|since| since.elapsed() < self.length(PHONE))
    }

    /// What just went up was put there by the phone.
    pub fn rung_by_phone(&self) -> bool {
        match (self.phone, &self.went_up) {
            (Some(phone), Some(up)) => up.since.saturating_duration_since(phone) < SOON,
            _ => false,
        }
    }

    /// The mode refused something, for the reason `why`.
    pub fn refuse(&mut self, why: &'static str) {
        let now = Instant::now();
        // The shake has a shape of its own: it goes evenly through it.
        let shake = Run::new(now, self.length(NUDGE), Easing::Linear);
        self.refused = Some((why, self.run(WHY, now), shake));
    }

    /// Why the last thing was refused, while it is being said, and how
    /// far its light has faded: `None` for the light with motion off.
    pub fn why(&self) -> Option<(&'static str, Option<f32>)> {
        let (why, light, _) = self.refused.as_ref()?;
        light
            .running(Instant::now())
            .then(|| (*why, self.along(Some(light))))
    }

    /// How far aside the note is, in pixels, as it shakes its head: out
    /// one way, back through the middle, less the other way, and still.
    pub fn nudge(&self) -> f32 {
        let head = self.refused.as_ref().map(|(_, _, head)| head);
        self.along(head).map_or(0.0, shake)
    }

    /// `step` was just added, replaced or moved.
    pub fn changed(&mut self, step: StepId) {
        self.changed = Some((step, self.run(FLASH, Instant::now())));
    }

    /// How far the flash of `step` has faded, while it has one.
    pub fn flash(&self, step: StepId) -> Option<f32> {
        let (changed, flash) = self.changed.as_ref()?;
        self.along((*changed == step).then_some(flash))
    }

    /// Lantern is in `mode` now.
    pub fn mode_is(&mut self, mode: Mode) {
        if mode != self.mode {
            self.left = Some((self.mode, self.run(SLIDE, Instant::now())));
            self.mode = mode;
        }
    }

    /// The mode the switch's pill is on its way from, and how far it has
    /// come.
    pub fn slide(&self) -> Option<(Mode, f32)> {
        let (from, slide) = self.left.as_ref()?;
        Some((*from, self.along(Some(slide))?))
    }

    /// What is carried would land at `landing` now. The gap it leaves
    /// closes as the new one opens, so between them the rows slide and the
    /// list keeps its height.
    pub fn lands_at(&mut self, landing: Option<usize>) {
        if landing != self.gap.as_ref().map(|(at, _)| *at) {
            let now = Instant::now();
            self.closing = self.gap.take().map(|(at, _)| (at, self.run(GAP, now)));
            self.gap = landing.map(|at| (at, self.run(GAP, now)));
        }
    }

    /// A row was just picked up from `home`: its gap is there, open, with
    /// nothing to wait for.
    pub fn lands_open(&mut self, home: usize) {
        let now = Instant::now();
        let before = now.checked_sub(self.length(GAP) * 2).unwrap_or(now);
        self.gap = Some((home, self.run(GAP, before)));
        self.closing = None;
    }

    /// Nothing is held: no gap, and none closing. What was dropped takes
    /// the gap's place as it is.
    pub fn nothing_held(&mut self) {
        self.gap = None;
        self.closing = None;
        self.indent = None;
        self.picked = None;
        self.host = None;
    }

    /// What is carried would be a part of `section` now, or of none.
    pub fn hosted_by(&mut self, section: Option<StepId>) {
        if section != self.host.as_ref().map(|(host, _)| *host) {
            self.host = section.map(|host| (host, self.run(LEVEL, Instant::now())));
        }
    }

    /// How far the section it would be a part of has lit up: 1 when lit.
    pub fn host_lit(&self) -> f32 {
        self.along(self.host.as_ref().map(|(_, run)| run))
            .unwrap_or(1.0)
    }

    /// Something was just picked up: its card rises under the pointer.
    pub fn picked_up(&mut self) {
        self.picked = Some(self.run(LIFT, Instant::now()));
    }

    /// A ring opens: its buttons pop out, one after another.
    pub fn ring_opens(&mut self) {
        self.ringing = Some(Run::new(
            Instant::now(),
            self.length(BUBBLE),
            Easing::Linear,
        ));
    }

    /// How far a ring has opened, evenly in time: 1 once it is open.
    pub fn ring_open(&self) -> f32 {
        self.along(self.ringing.as_ref()).unwrap_or(1.0)
    }

    /// How far the card of what is carried has risen: 1 once it is up.
    pub fn lift(&self) -> f32 {
        self.along(self.picked.as_ref()).unwrap_or(1.0)
    }

    /// The gap is `indent` pixels in now. It slides there from where it
    /// is, so going in or out a level is seen to happen.
    pub fn gap_is_in(&mut self, indent: f32) {
        if self.indent.as_ref().is_some_and(|(_, to, _)| *to == indent) {
            return;
        }
        let from = self.gap_indent().unwrap_or(indent);
        self.indent = Some((from, indent, self.run(LEVEL, Instant::now())));
    }

    /// How far in the gap is drawn, on its way to where it is going.
    pub fn gap_indent(&self) -> Option<f32> {
        let (from, to, slide) = self.indent.as_ref()?;
        Some(match self.along(Some(slide)) {
            Some(along) => between(*from, *to, along),
            None => *to,
        })
    }

    /// No gap is open or closing: a drag that starts now starts fresh.
    pub fn no_gap(&self) -> bool {
        self.gap.is_none() && self.closing.is_none()
    }

    /// The gap that is closing: where, and how much of it is left.
    pub fn closing_gap(&self) -> Option<(usize, f32)> {
        let (at, closing) = self.closing.as_ref()?;
        Some((*at, 1.0 - self.along(Some(closing))?))
    }

    /// How far open the gap is, while it opens.
    pub fn gap(&self) -> Option<f32> {
        self.along(self.gap.as_ref().map(|(_, gap)| gap))
    }

    /// Something is moving: every frame is drawn.
    pub fn moving(&self) -> bool {
        let now = Instant::now();
        let runs: [Option<&Run>; 20] = [
            self.flight.as_ref().map(|(_, _, _, run)| run),
            self.proposed.as_ref().map(|(_, run)| run),
            self.accepted.as_ref().map(|(_, run)| run),
            self.tipped.as_ref().map(|(_, run)| run),
            self.round.as_ref().map(|(_, run)| run),
            self.stepped.as_ref().map(|(_, run)| run),
            self.gone.as_ref().map(|(_, _, run)| run),
            self.came.as_ref().map(|(_, run)| run),
            self.saved.as_ref(),
            self.breath.as_ref(),
            self.went_up.as_ref(),
            self.left.as_ref().map(|(_, run)| run),
            self.gap.as_ref().map(|(_, run)| run),
            self.closing.as_ref().map(|(_, run)| run),
            self.indent.as_ref().map(|(_, _, run)| run),
            self.picked.as_ref(),
            self.ringing.as_ref(),
            self.host.as_ref().map(|(_, run)| run),
            self.refused.as_ref().map(|(_, light, _)| light),
            self.changed.as_ref().map(|(_, run)| run),
        ];
        !self.still && runs.into_iter().flatten().any(|run| run.running(now))
    }

    /// Something that shows for a while is up: looked at now and then, to
    /// take it down on time.
    pub fn showing(&self) -> bool {
        self.by_phone()
            || self
                .refused
                .as_ref()
                .is_some_and(|(_, light, _)| light.running(Instant::now()))
    }
}

/// A shake of the head over `t` from 0 to 1: five pixels one way, back,
/// fewer the other way, and still at the end.
fn shake(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    -5.0 * (t * std::f32::consts::TAU).sin() * (1.0 - t)
}

/// `from` to `to`, `t` of the way, each part of the colour by itself.
pub fn blend(from: iced::Color, to: iced::Color, t: f32) -> iced::Color {
    iced::Color {
        r: between(from.r, to.r, t),
        g: between(from.g, to.g, t),
        b: between(from.b, to.b, t),
        a: between(from.a, to.a, t),
    }
}

/// `from` to `to`, `t` of the way.
pub fn between(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Fast at first, slowing to a stop.
pub fn ease(far: f32) -> f32 {
    1.0 - (1.0 - far.clamp(0.0, 1.0)).powi(3)
}

/// A number on its way from where it was to where it should be: evenly,
/// or fast at first and settling.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    from: f32,
    pub to: f32,
    since: Instant,
    length: Duration,
    eased: bool,
}

impl Glide {
    /// Already where it should be.
    pub fn at(to: f32, now: Instant) -> Glide {
        Glide {
            from: to,
            to,
            since: now,
            length: Duration::ZERO,
            eased: false,
        }
    }

    /// From `from` to `to` over `length`, set off at `now`.
    pub fn new(from: f32, to: f32, now: Instant, length: Duration, eased: bool) -> Glide {
        Glide {
            from,
            to,
            since: now,
            length,
            eased,
        }
    }

    /// From where it is at `now` on to `to` instead.
    pub fn towards(self, to: f32, now: Instant, length: Duration, eased: bool) -> Glide {
        Glide::new(self.shown(now), to, now, length, eased)
    }

    /// How far along its way it is, 0 to 1.
    fn far(&self, now: Instant) -> f32 {
        match self.length.is_zero() {
            true => 1.0,
            false => (now.saturating_duration_since(self.since).as_secs_f32()
                / self.length.as_secs_f32())
            .clamp(0.0, 1.0),
        }
    }

    /// Where it is shown at `now`.
    pub fn shown(&self, now: Instant) -> f32 {
        let far = self.far(now);
        between(self.from, self.to, if self.eased { ease(far) } else { far })
    }

    pub fn moving(&self, now: Instant) -> bool {
        self.far(now) < 1.0
    }

    /// When it will have gone `least` further than where it is at `now`:
    /// at once while eased, as it goes fast at first. `None` once there.
    pub fn when_moved(&self, least: f32, now: Instant) -> Option<Instant> {
        self.moving(now).then_some(())?;
        if self.eased {
            return Some(now);
        }
        let speed = (self.to - self.from).abs() / self.length.as_secs_f32();
        let wait = Duration::try_from_secs_f32(least / speed).unwrap_or(self.length);
        Some((now + wait).min(self.since + self.length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_starts_fast_and_ends_at_rest() {
        let start = Instant::now();
        let run = Run::new(start, Duration::from_secs(1), Easing::EaseOutCubic);
        assert_eq!(run.along(start), Some(0.0));
        let half = run.along(start + Duration::from_millis(500)).unwrap();
        assert!(half > 0.5, "more than half done at half time: {half}");
        assert_eq!(run.along(start + Duration::from_secs(2)), None, "over");
    }

    #[test]
    fn what_went_up_rings_once_and_then_is_over() {
        let mut motion = Motion::new();
        assert!(motion.ring().is_none());
        motion.went_up = Some(motion.run(RING, Instant::now()));
        assert!(motion.ring().is_some());
        assert!(motion.moving());
        motion.went_up = Instant::now()
            .checked_sub(RING * 2)
            .map(|since| motion.run(RING, since));
        assert!(motion.ring().is_none());
        assert!(!motion.moving());
    }

    #[test]
    fn a_glide_goes_its_way_and_stays_there() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let even = Glide::new(0.2, 0.6, start, second, false);
        assert_eq!(even.shown(start), 0.2);
        let half = even.shown(start + second / 2);
        assert!((half - 0.4).abs() < 1e-5, "evenly: {half}");
        assert_eq!(even.shown(start + second * 3), 0.6, "no further");
        assert!(even.moving(start) && !even.moving(start + second));
        // Turned on its way, it goes on from where it is, not from
        // where it began.
        let turned = even.towards(0.0, start + second / 2, second, true);
        assert!((turned.shown(start + second / 2) - 0.4).abs() < 1e-5);
        let soon = turned.shown(start + second);
        assert!(soon < 0.2, "fast at first: {soon}");
        assert_eq!(turned.shown(start + second * 2), 0.0);
        // Evenly, it is looked at again once it has gone far enough to
        // be seen to, and no later than its end.
        let next = even.when_moved(0.04, start).unwrap() - start;
        assert!(next.abs_diff(Duration::from_millis(100)) < Duration::from_millis(1));
        assert_eq!(
            even.when_moved(1.0, start),
            Some(start + second),
            "not past its end"
        );
        assert_eq!(
            turned.when_moved(0.04, start + second),
            Some(start + second)
        );
        assert_eq!(even.when_moved(0.04, start + second), None);
        let level = Glide::new(0.3, 0.3, start, second, false);
        assert_eq!(level.when_moved(0.04, start), Some(start + second));
        // Already there, it does not move.
        assert!(!Glide::at(0.5, start).moving(start));
        assert_eq!(Glide::at(0.5, start).when_moved(0.04, start), None);
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
    }

    #[test]
    fn going_on_rings_what_went_on_and_fades_what_was_on() {
        let mut motion = Motion::new();
        let mut plan = lantern_engine::Plan::new("Sábado");
        let a = plan.push("A", lantern_engine::Action::Blank);
        let b = plan.push("B", lantern_engine::Action::Blank);
        motion.went_on(Some(a));
        assert!(motion.stepped().is_some() && motion.moving());
        assert!(motion.going_off(a).is_some());
        assert!(motion.going_off(b).is_none());
        motion.still = true;
        assert!(motion.stepped().is_none() && motion.going_off(a).is_none());
    }

    #[test]
    fn a_refusal_shakes_both_ways_and_ends_still() {
        assert_eq!(shake(0.0), 0.0);
        assert!(shake(0.25) < -3.0, "out one way first");
        assert!(shake(0.75) > 0.5, "then less the other way");
        assert!(shake(0.75) < -shake(0.25));
        assert_eq!(shake(1.0), 0.0);

        let mut motion = Motion::new();
        assert!(motion.why().is_none());
        motion.refuse("the plan is locked while it runs");
        assert_eq!(
            motion.why().map(|(why, _)| why),
            Some("the plan is locked while it runs")
        );
        // Said with motion off too, with nothing moving.
        motion.still = true;
        assert_eq!(motion.nudge(), 0.0);
        assert!(matches!(motion.why(), Some((_, None))));
        assert!(motion.showing() && !motion.moving());
    }

    #[test]
    fn the_pill_slides_from_the_mode_it_left() {
        let mut motion = Motion::new();
        motion.mode_is(Mode::Plan);
        assert!(motion.slide().is_none(), "it started there");
        motion.mode_is(Mode::Live);
        assert!(matches!(motion.slide(), Some((Mode::Plan, _))));
        motion.mode_is(Mode::Live);
        assert!(
            matches!(motion.slide(), Some((Mode::Plan, _))),
            "not started over"
        );
    }

    #[test]
    fn the_gap_opens_again_only_where_it_moved_to() {
        let mut motion = Motion::new();
        motion.lands_at(Some(2));
        let since = motion.gap.as_ref().map(|(_, gap)| gap.since);
        motion.lands_at(Some(2));
        assert_eq!(motion.gap.as_ref().map(|(_, gap)| gap.since), since);
        motion.lands_at(Some(3));
        assert_eq!(motion.gap.as_ref().map(|(at, _)| *at), Some(3));
        // The one it left closes as the new one opens.
        assert!(matches!(motion.closing_gap(), Some((2, left)) if left > 0.5));
        motion.lands_at(None);
        assert!(motion.gap().is_none());
        assert!(matches!(motion.closing_gap(), Some((3, _))));
        // Let go: what was dropped is there, with no gap closing beside it.
        motion.nothing_held();
        assert!(motion.no_gap());
        // Picked up: the gap is where the row was, open already.
        motion.lands_open(4);
        motion.picked_up();
        assert!(motion.lift() < 1.0 && motion.moving());
        // It goes in a level: on its way there from where it was.
        motion.gap_is_in(0.0);
        assert_eq!(motion.gap_indent(), Some(0.0), "it starts where it is");
        motion.gap_is_in(12.0);
        let sliding = motion.gap_indent().unwrap();
        assert!((0.0..12.0).contains(&sliding), "{sliding}");
        motion.nothing_held();
        assert_eq!(motion.gap_indent(), None);
        motion.lands_open(4);
        assert!(motion.gap().is_none() && motion.closing_gap().is_none());
        assert!(!motion.no_gap());
    }

    #[test]
    fn with_motion_off_nothing_moves_but_the_phone_is_still_named() {
        let mut motion = Motion::new();
        motion.still = true;
        motion.phone_acted();
        motion.went_up = Some(motion.run(RING, Instant::now()));
        assert!(motion.ring().is_none());
        assert!(!motion.moving());
        assert!(motion.by_phone());
        assert!(motion.rung_by_phone());
    }

    #[test]
    fn a_ring_s_buttons_pop_out_one_after_another_past_their_place_and_back() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(near(bubble(0.0, 0, 4), 0.0));
        assert!(near(bubble(1.0, 3, 4), 1.0));
        // The first is out before the last has started.
        assert!(bubble(0.3, 0, 4) > 0.5 && near(bubble(0.3, 3, 4), 0.0));
        // Somewhere on its way it goes past where it settles.
        assert!((0..=100).any(|k| bubble(k as f32 / 100.0, 1, 4) > 1.05));
    }
}
