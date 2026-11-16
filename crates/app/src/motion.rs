//! The dashboard's motion: what moves for a moment to say that something
//! happened, and then stops.
//!
//! Every one is short and plays once. Nothing loops: the booth is dark, and
//! a screen that keeps pulsing pulls the eye from the stage. None delays
//! what it follows either; the projector has changed before its ring is
//! drawn.
//!
//! Each is kept as the moment it started. The view asks how far along it
//! is, and the app asks for frames only while one is running.

use std::time::{Duration, Instant};

use lantern_engine::{SceneId, StepId};

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
/// A section's steps closing under its heading, or opening.
pub const FOLD: Duration = Duration::from_millis(180);
/// The gap opening where what is carried would land.
pub const GAP: Duration = Duration::from_millis(150);
/// The phone's press and what it put on stage are this close together.
const SOON: Duration = Duration::from_millis(1500);

pub struct Motion {
    /// Nothing moves: every change is drawn as it ends up.
    pub still: bool,
    /// Every length is this many times longer, to look at one closely.
    slow: f32,
    /// What is on the projector, and when a new thing last went up.
    program: Option<SceneId>,
    went_up: Option<Instant>,
    /// When the phone last did something to the stage.
    phone: Option<Instant>,
    /// What was last refused, in the mode's words, and when.
    refused: Option<(&'static str, Instant)>,
    /// The step that last changed, and when.
    changed: Option<(StepId, Instant)>,
    /// The mode it is in, the one it was in before, and when it changed.
    mode: Mode,
    left: Option<(Mode, Instant)>,
    /// The section last folded (`true`) or opened, and when.
    fold: Option<(StepId, bool, Instant)>,
    /// Where what is carried would land, and since when.
    gap: Option<(usize, Instant)>,
    /// Where it would have landed before that, closing since then.
    closing: Option<(usize, Instant)>,
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
            fold: None,
            gap: None,
            closing: None,
        }
    }

    fn length(&self, of: Duration) -> Duration {
        of.mul_f32(self.slow)
    }

    /// How far along something that started at `since` and takes `length`
    /// is, eased so it starts fast and settles: 0 at its start, and `None`
    /// once it is over, or with motion off.
    fn along(&self, since: Option<Instant>, length: Duration) -> Option<f32> {
        if self.still {
            return None;
        }
        let length = self.length(length);
        let passed = since?.elapsed();
        (passed < length).then(|| ease(passed.as_secs_f32() / length.as_secs_f32()))
    }

    fn running(&self, since: Option<Instant>, length: Duration) -> bool {
        since.is_some_and(|since| since.elapsed() < self.length(length))
    }

    /// The projector shows `program` now. Something new going up rings.
    pub fn stage_shows(&mut self, program: Option<SceneId>) {
        if program != self.program {
            self.program = program;
            self.went_up = program.map(|_| Instant::now());
        }
    }

    /// The ring of what just went on the projector.
    pub fn ring(&self) -> Option<f32> {
        self.along(self.went_up, RING)
    }

    /// The phone did something to the stage.
    pub fn phone_acted(&mut self) {
        self.phone = Some(Instant::now());
    }

    /// The stage was last moved from the phone, a moment ago. Said with
    /// motion off too: it is something to know, not something moving.
    pub fn by_phone(&self) -> bool {
        self.running(self.phone, PHONE)
    }

    /// What just went up was put there by the phone.
    pub fn rung_by_phone(&self) -> bool {
        match (self.phone, self.went_up) {
            (Some(phone), Some(up)) => up.saturating_duration_since(phone) < SOON,
            _ => false,
        }
    }

    /// The mode refused something, for the reason `why`.
    pub fn refuse(&mut self, why: &'static str) {
        self.refused = Some((why, Instant::now()));
    }

    /// Why the last thing was refused, while it is being said, and how
    /// far its light has faded: `None` for the light with motion off.
    pub fn why(&self) -> Option<(&'static str, Option<f32>)> {
        let (why, at) = self.refused?;
        self.running(Some(at), WHY)
            .then(|| (why, self.along(Some(at), WHY)))
    }

    /// How far aside the note is, in pixels, as it shakes its head: out
    /// one way, back through the middle, less the other way, and still.
    pub fn nudge(&self) -> f32 {
        if self.still {
            return 0.0;
        }
        let Some((_, at)) = self.refused else {
            return 0.0;
        };
        let length = self.length(NUDGE);
        let passed = at.elapsed();
        if passed >= length {
            return 0.0;
        }
        shake(passed.as_secs_f32() / length.as_secs_f32())
    }

    /// `step` was just added, replaced or moved.
    pub fn changed(&mut self, step: StepId) {
        self.changed = Some((step, Instant::now()));
    }

    /// How far the flash of `step` has faded, while it has one.
    pub fn flash(&self, step: StepId) -> Option<f32> {
        let (changed, at) = self.changed?;
        self.along((changed == step).then_some(at), FLASH)
    }

    /// Lantern is in `mode` now.
    pub fn mode_is(&mut self, mode: Mode) {
        if mode != self.mode {
            self.left = Some((self.mode, Instant::now()));
            self.mode = mode;
        }
    }

    /// The mode the switch's pill is on its way from, and how far it has
    /// come.
    pub fn slide(&self) -> Option<(Mode, f32)> {
        let (from, at) = self.left?;
        Some((from, self.along(Some(at), SLIDE)?))
    }

    /// `section` was folded, or opened.
    pub fn folded(&mut self, section: StepId, closed: bool) {
        self.fold = Some((section, closed, Instant::now()));
    }

    /// How much of `section`'s steps is in sight while they close or
    /// open: 1 is all of them.
    pub fn open(&self, section: StepId) -> Option<f32> {
        let (folding, closed, at) = self.fold?;
        let along = self.along((folding == section).then_some(at), FOLD)?;
        Some(if closed { 1.0 - along } else { along })
    }

    /// What is carried would land at `landing` now. The gap it leaves
    /// closes as the new one opens, so between them the rows slide and the
    /// list keeps its height.
    pub fn lands_at(&mut self, landing: Option<usize>) {
        if landing != self.gap.map(|(at, _)| at) {
            let now = Instant::now();
            self.closing = self.gap.take().map(|(at, _)| (at, now));
            self.gap = landing.map(|at| (at, now));
        }
    }

    /// A row was just picked up from `home`: its gap is there, open, with
    /// nothing to wait for.
    pub fn lands_open(&mut self, home: usize) {
        let now = Instant::now();
        let before = now.checked_sub(self.length(GAP) * 2).unwrap_or(now);
        self.gap = Some((home, before));
        self.closing = None;
    }

    /// Nothing is held: no gap, and none closing. What was dropped takes
    /// the gap's place as it is.
    pub fn nothing_held(&mut self) {
        self.gap = None;
        self.closing = None;
    }

    /// No gap is open or closing: a drag that starts now starts fresh.
    pub fn no_gap(&self) -> bool {
        self.gap.is_none() && self.closing.is_none()
    }

    /// The gap that is closing: where, and how much of it is left.
    pub fn closing_gap(&self) -> Option<(usize, f32)> {
        let (at, since) = self.closing?;
        Some((at, 1.0 - self.along(Some(since), GAP)?))
    }

    /// How far open the gap is, while it opens.
    pub fn gap(&self) -> Option<f32> {
        self.along(self.gap.map(|(_, at)| at), GAP)
    }

    /// Something is moving: every frame is drawn.
    pub fn moving(&self) -> bool {
        !self.still
            && (self.running(self.went_up, RING)
                || self.running(self.left.map(|(_, at)| at), SLIDE)
                || self.running(self.fold.map(|(_, _, at)| at), FOLD)
                || self.running(self.gap.map(|(_, at)| at), GAP)
                || self.running(self.closing.map(|(_, at)| at), GAP)
                || self.running(self.refused.map(|(_, at)| at), WHY)
                || self.running(self.changed.map(|(_, at)| at), FLASH))
    }

    /// Something that shows for a while is up: looked at now and then, to
    /// take it down on time.
    pub fn showing(&self) -> bool {
        self.by_phone() || self.running(self.refused.map(|(_, at)| at), WHY)
    }
}

/// Fast at first, settling at the end.
fn ease(t: f32) -> f32 {
    let left = 1.0 - t.clamp(0.0, 1.0);
    1.0 - left * left * left
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_starts_fast_and_ends_at_rest() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert!(ease(0.5) > 0.5, "more than half done at half time");
        assert_eq!(ease(2.0), 1.0, "never past the end");
    }

    #[test]
    fn what_went_up_rings_once_and_then_is_over() {
        let mut motion = Motion::new();
        assert!(motion.ring().is_none());
        motion.went_up = Some(Instant::now());
        assert!(motion.ring().is_some());
        assert!(motion.moving());
        motion.went_up = Instant::now().checked_sub(RING * 2);
        assert!(motion.ring().is_none());
        assert!(!motion.moving());
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
        let since = motion.gap;
        motion.lands_at(Some(2));
        assert_eq!(motion.gap, since);
        motion.lands_at(Some(3));
        assert_eq!(motion.gap.map(|(at, _)| at), Some(3));
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
        assert!(motion.gap().is_none() && motion.closing_gap().is_none());
        assert!(!motion.no_gap());
    }

    #[test]
    fn with_motion_off_nothing_moves_but_the_phone_is_still_named() {
        let mut motion = Motion::new();
        motion.still = true;
        motion.phone_acted();
        motion.went_up = Some(Instant::now());
        assert!(motion.ring().is_none());
        assert!(!motion.moving());
        assert!(motion.by_phone());
        assert!(motion.rung_by_phone());
    }
}
