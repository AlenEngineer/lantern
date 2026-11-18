//! Dragging plan rows: what a drag carries and where it lands.
//!
//! A step moves alone; a section brings its steps. While a drag moves, the
//! carried rows leave the list and a gap opens where they'd land, so
//! positions here count the plan *without* them (the "rest").
//!
//! Up and down say where; for a section, left and right say how far in:
//! its own section, or a part of the one before it. A step has no such
//! choice: it is in whatever section it lands under.

use iced::Point;
use lantern_engine::{DEEPEST, Plan, Step, StepId};

/// Which half of a row the pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Half {
    Upper,
    Lower,
}

/// A row held down with the left button.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    /// The row pressed on.
    pub step: StepId,
    /// What moves with it.
    pub carried: Vec<StepId>,
    /// Whether the row is a section's header.
    section: bool,
    /// Where it was picked up from, as an index in the rest: let go
    /// there, nothing has moved.
    pub home: usize,
    /// Where the pointer was when it first moved after the press.
    origin: Option<Point>,
    /// Where the pointer is, in the dashboard window.
    pub at: Point,
    /// Whether it has moved far enough to be a drag, not a click.
    pub moving: bool,
    /// Where the carried rows would land if let go now, as an index in
    /// the rest; `None` until the pointer is over a row, and after it
    /// leaves the plan.
    pub landing: Option<usize>,
    /// A section's own level when it was picked up, and how much further
    /// in the deepest section it carries is.
    own: u8,
    span: u8,
    /// The level it would land at: for a section its own depth there,
    /// for a step how far in its row would be drawn.
    pub level: u8,
    /// The least and the most a section's level can be where it would
    /// land.
    pub levels: (u8, u8),
}

/// How far in a heading sits.
fn heading(step: &Step) -> Option<u8> {
    step.depth()
}

/// Whether landing at `at` among `rest` is among steps: a step comes
/// right after it.
fn among(rest: &[&Step], at: usize) -> bool {
    rest.get(at).is_some_and(|step| heading(step).is_none())
}

/// Puts the section `carried` (its heading first, then all it holds) at
/// `at` among the rows that stay, `level` deep, as [`Plan::move_group`]
/// does. Landing `among` steps, it ends after its own rows, so the steps
/// after it stay in the section around it. Whether anything moved.
pub fn place_section(
    plan: &mut Plan,
    carried: &[StepId],
    at: usize,
    level: u8,
    among: bool,
) -> bool {
    if !plan.move_group(carried, at, level) {
        return false;
    }
    if among && let (Some(section), Some(last)) = (carried.first(), carried.last()) {
        let _ = plan.reach_section(*section, Some(*last));
    }
    true
}

/// How far the pointer goes sideways for one level in or out.
const LEVEL_STEP: f32 = 24.0;

/// How far in a step's row is drawn, landing at `at` among `rest`: one
/// further than the section it lands under.
pub fn level_at(rest: &[&Step], at: usize) -> u8 {
    rest[..at.min(rest.len())]
        .iter()
        .rev()
        .find_map(|step| heading(step))
        .map_or(0, |depth| depth + 1)
}

/// The section something landing at `at` among `rest`, drawn at `level`,
/// would be a part of: where it is in the rest, and where what it holds
/// ends. `None` at the plan's own level.
pub fn host(rest: &[&Step], at: usize, level: u8) -> Option<(usize, usize)> {
    let above = level.checked_sub(1)?;
    let start = rest[..at.min(rest.len())]
        .iter()
        .rposition(|step| heading(step).is_some_and(|depth| depth <= above))
        .filter(|start| heading(rest[*start]) == Some(above))?;
    let end = rest[start + 1..]
        .iter()
        .position(|step| heading(step).is_some_and(|depth| depth <= above))
        .map_or(rest.len(), |n| start + 1 + n);
    Some((start, end))
}

/// How far the pointer moves before a press becomes a drag.
const SLOP: f32 = 5.0;

impl Drag {
    pub fn new(plan: &Plan, step: StepId) -> Drag {
        let carried = plan.carried(step);
        let depth = |id: &StepId| plan.get(*id).and_then(Step::depth);
        let own = depth(&step).unwrap_or(0);
        let deepest = carried.iter().filter_map(depth).max().unwrap_or(own);
        Drag {
            step,
            own,
            span: deepest - own,
            level: own,
            levels: (own, own),
            carried,
            section: plan
                .steps()
                .iter()
                .find(|s| s.id == step)
                .is_some_and(|s| s.is_section()),
            home: plan.position(step).unwrap_or(0),
            origin: None,
            at: Point::ORIGIN,
            moving: false,
            landing: None,
        }
    }

    /// Whether it carries a section (and its steps).
    pub fn is_section(&self) -> bool {
        self.section
    }

    /// What it carries, in words: "Hymn", or "Worship · 3 steps".
    pub fn label(&self, plan: &Plan) -> String {
        let name = plan
            .steps()
            .iter()
            .find(|step| step.id == self.step)
            .map_or_else(String::new, |step| step.label.clone());
        match self.carried.len() - 1 {
            0 if !self.section => name,
            more if !self.section => format!("{name} + {more}"),
            0 => format!("{name} · empty"),
            1 => format!("{name} · 1 step"),
            n => format!("{name} · {n} steps"),
        }
    }

    /// The pointer moved to `at`.
    pub fn to(&mut self, at: Point) {
        let origin = *self.origin.get_or_insert(at);
        self.at = at;
        if !self.moving && origin.distance(at) > SLOP {
            self.moving = true;
            // It starts over the place it left, so the gap is there and
            // the rows under it don't jump up to fill it.
            self.landing.get_or_insert(self.home);
        }
    }

    /// The plan without what is carried.
    pub fn rest<'a>(&self, plan: &'a Plan) -> Vec<&'a Step> {
        plan.steps()
            .iter()
            .filter(|step| !self.carried.contains(&step.id))
            .collect()
    }

    /// Works out the level it would land at, from where it would land
    /// and how far sideways the pointer has gone since it was picked up.
    ///
    /// A section can be at the plan's own level or a part of the section
    /// before it, no further in than that one's parts are, and no further
    /// than leaves room for the sections it carries. It is never further
    /// out than the section after it: that one would become its part.
    pub fn aim_level(&mut self, plan: &Plan) {
        let Some(at) = self.landing else {
            return;
        };
        let rest = self.rest(plan);
        if !self.section {
            self.level = level_at(&rest, at);
            self.levels = (self.level, self.level);
            return;
        }
        // Among steps: in the section that holds them, no further.
        if among(&rest, at) {
            self.level = level_at(&rest, at).min(DEEPEST.saturating_sub(self.span));
            self.levels = (self.level, self.level);
            return;
        }
        let before = rest[..at.min(rest.len())]
            .iter()
            .rev()
            .find_map(|step| heading(step));
        let after = rest.get(at).and_then(|step| heading(step)).unwrap_or(0);
        let most = before
            .map_or(0, |before| before + 1)
            .min(DEEPEST.saturating_sub(self.span));
        let least = after.min(most);
        let sideways = self.origin.map_or(0.0, |origin| self.at.x - origin.x);
        let wish = f32::from(self.own) + (sideways / LEVEL_STEP).round();
        self.level = wish.clamp(f32::from(least), f32::from(most)) as u8;
        self.levels = (least, most);
    }

    /// Whether a section carried would land among steps, where it is put
    /// in the section that holds them and holds only its own.
    pub fn among(&self, plan: &Plan) -> bool {
        self.section && self.landing.is_some_and(|at| among(&self.rest(plan), at))
    }

    /// Whether letting go would change how far in a section is, even
    /// where it was picked up.
    pub fn nests(&self) -> bool {
        self.section && self.level != self.own
    }

    /// Where the carried rows land with the pointer over `half` of row
    /// `over`, as an index in the rest; `None` over a carried row.
    ///
    /// A step goes above or below the row. A section goes between sections
    /// only, so it never splits one: above the section it's over when on
    /// the upper half of its header, otherwise after its last step.
    pub fn landing(&self, plan: &Plan, over: StepId, half: Half) -> Option<usize> {
        let rest: Vec<_> = plan
            .steps()
            .iter()
            .filter(|step| !self.carried.contains(&step.id))
            .collect();
        let at = rest.iter().position(|step| step.id == over)?;
        if !self.section {
            return Some(at + usize::from(half == Half::Lower));
        }
        if heading(rest[at]).is_some() && half == Half::Upper {
            return Some(at);
        }
        // Over a step it lands among the steps, before or after it: it is
        // put in the section that holds them, holding only its own.
        if heading(rest[at]).is_none() {
            return Some(at + usize::from(half == Half::Lower));
        }
        let end = rest[at + 1..]
            .iter()
            .position(|step| heading(step).is_some())
            .map_or(rest.len(), |n| at + 1 + n);
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use lantern_engine::{Kind, Location, Resource, ResourceId, SourceId};

    use super::*;

    fn resource(title: &str) -> Resource {
        let source = SourceId::new("test").unwrap();
        Resource::new(
            ResourceId::new(&source, title),
            title,
            Kind::Image,
            Location::Url(title.into()),
        )
    }

    /// Welcome; Worship [Song, Hymn]; Word [Reading].
    fn sunday() -> (Plan, [StepId; 6]) {
        let mut plan = Plan::new("Sunday");
        let ids = [
            plan.show(resource("Welcome")),
            plan.section("Worship", None),
            plan.show(resource("Song")),
            plan.show(resource("Hymn")),
            plan.section("Word", None),
            plan.show(resource("Reading")),
        ];
        (plan, ids)
    }

    fn labels(plan: &Plan) -> Vec<&str> {
        plan.steps().iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn a_press_becomes_a_drag_only_after_the_pointer_moves() {
        let (plan, [_, _, song, ..]) = sunday();
        let mut drag = Drag::new(&plan, song);
        drag.to(Point::new(100.0, 100.0));
        drag.to(Point::new(102.0, 103.0));
        assert!(!drag.moving, "a shaky click stays a click");
        drag.to(Point::new(100.0, 120.0));
        assert!(drag.moving);
    }

    #[test]
    fn a_step_lands_above_or_below_the_row() {
        let (mut plan, [welcome, worship, song, _, word, reading]) = sunday();
        let drag = Drag::new(&plan, welcome);
        // Rest: Worship, Song, Hymn, Word, Reading.
        assert_eq!(drag.landing(&plan, song, Half::Upper), Some(1));
        assert_eq!(drag.landing(&plan, song, Half::Lower), Some(2));
        assert_eq!(
            drag.landing(&plan, word, Half::Upper),
            Some(3),
            "end of Worship"
        );
        assert_eq!(
            drag.landing(&plan, word, Half::Lower),
            Some(4),
            "first in Word"
        );
        assert_eq!(
            drag.landing(&plan, welcome, Half::Lower),
            None,
            "over itself"
        );
        let at = drag.landing(&plan, reading, Half::Lower).unwrap();
        plan.move_steps(&drag.carried, at);
        assert_eq!(
            labels(&plan),
            ["Worship", "Song", "Hymn", "Word", "Reading", "Welcome"]
        );
        let _ = worship;
    }

    #[test]
    fn a_section_lands_between_sections_or_among_steps_with_its_steps() {
        let (mut plan, [welcome, worship, song, _, word, reading]) = sunday();
        let drag = Drag::new(&plan, worship);
        assert_eq!(drag.carried.len(), 3);
        assert_eq!(drag.label(&plan), "Worship · 2 steps");
        assert_eq!(Drag::new(&plan, song).label(&plan), "Song");
        // Rest: Welcome, Word, Reading.
        assert_eq!(drag.landing(&plan, word, Half::Upper), Some(1));
        assert_eq!(drag.landing(&plan, word, Half::Lower), Some(3));
        assert_eq!(
            drag.landing(&plan, reading, Half::Upper),
            Some(2),
            "among Word's steps, before Reading"
        );
        assert_eq!(
            drag.landing(&plan, welcome, Half::Upper),
            Some(0),
            "among the loose steps, before Welcome"
        );
        assert_eq!(
            drag.landing(&plan, song, Half::Upper),
            None,
            "over its own step"
        );
        let at = drag.landing(&plan, reading, Half::Lower).unwrap();
        plan.move_steps(&drag.carried, at);
        assert_eq!(
            labels(&plan),
            ["Welcome", "Word", "Reading", "Worship", "Song", "Hymn"]
        );
    }

    /// Singing [Hymn]; Special [Duet]; School [Reading].
    fn sabbath() -> (Plan, [StepId; 6]) {
        let mut plan = Plan::new("Sabbath");
        let ids = [
            plan.section("Singing", None),
            plan.show(resource("Hymn")),
            plan.section("Special", None),
            plan.show(resource("Duet")),
            plan.section("School", None),
            plan.show(resource("Reading")),
        ];
        (plan, ids)
    }

    #[test]
    fn sideways_a_section_goes_into_the_one_before_it_and_out_again() {
        let (mut plan, [singing, _, special, _, school, _]) = sabbath();
        let mut drag = Drag::new(&plan, special);
        drag.to(Point::new(100.0, 100.0));
        drag.to(Point::new(100.0, 110.0));
        assert!(drag.moving);
        drag.aim_level(&plan);
        // Where it was: its own section, or a part of Singing.
        assert_eq!((drag.level, drag.levels), (0, (0, 1)));
        assert!(!drag.nests());
        // A step's worth to the right: into Singing.
        drag.to(Point::new(100.0 + LEVEL_STEP, 110.0));
        drag.aim_level(&plan);
        assert_eq!(drag.level, 1);
        assert!(drag.nests(), "let go where it is, it still moves in");
        let rest = drag.rest(&plan);
        let at = drag.landing.unwrap();
        assert_eq!(host(&rest, at, drag.level), Some((0, 2)), "Singing");
        // Further right changes nothing: Singing has no part to be in.
        drag.to(Point::new(100.0 + 5.0 * LEVEL_STEP, 110.0));
        drag.aim_level(&plan);
        assert_eq!(drag.level, 1);
        plan.move_group(&drag.carried, at, drag.level);
        assert_eq!(plan.within(plan.position(special).unwrap()), [singing]);
        // Picked up again and pulled left, it is its own section again.
        let mut drag = Drag::new(&plan, special);
        drag.to(Point::new(100.0, 100.0));
        drag.to(Point::new(100.0 - LEVEL_STEP, 110.0));
        drag.aim_level(&plan);
        assert_eq!(drag.level, 0);
        // First in the plan there is nothing to be a part of.
        drag.landing = Some(0);
        drag.to(Point::new(200.0, 110.0));
        drag.aim_level(&plan);
        assert_eq!((drag.level, drag.levels), (0, (0, 0)));
        let _ = school;
    }

    #[test]
    fn a_section_is_never_further_out_than_the_part_after_it() {
        let (mut plan, [_, _, special, _, school, _]) = sabbath();
        // Special and School are parts of Singing.
        plan.move_group(&plan.carried(special), 2, 1);
        plan.move_group(&plan.carried(school), 4, 1);
        let mut drag = Drag::new(&plan, school);
        drag.to(Point::new(100.0, 100.0));
        drag.to(Point::new(100.0, 110.0));
        // Before Special, which is a part: out at the plan's level it
        // would take Special for its own.
        drag.landing = Some(2);
        drag.to(Point::new(100.0 - 3.0 * LEVEL_STEP, 110.0));
        drag.aim_level(&plan);
        assert_eq!((drag.level, drag.levels), (1, (1, 1)));
    }

    #[test]
    fn a_step_is_as_far_in_as_what_it_lands_under() {
        let (mut plan, [_, hymn, special, duet, ..]) = sabbath();
        plan.move_group(&plan.carried(special), 2, 1);
        let mut drag = Drag::new(&plan, hymn);
        drag.to(Point::new(100.0, 100.0));
        drag.to(Point::new(100.0, 110.0));
        // Rest: Singing, Special, Duet, School, Reading. Under Duet.
        drag.landing = drag.landing(&plan, duet, Half::Lower);
        drag.aim_level(&plan);
        assert_eq!(drag.level, 2);
        let rest = drag.rest(&plan);
        assert_eq!(host(&rest, 3, 2), Some((1, 3)), "Special, to its end");
        assert_eq!(host(&rest, 0, 0), None, "the plan's own level");
    }

    #[test]
    fn a_section_landed_among_steps_holds_only_its_own() {
        let (mut plan, [_, worship, song, hymn, word, reading]) = sunday();
        let more = plan.show(resource("More"));
        // Welcome; Worship [Song, Hymn]; Word [Reading, More].
        let mut drag = Drag::new(&plan, worship);
        // Rest: Welcome, Word, Reading, More. Over Reading's lower half:
        // between Reading and More, in Word.
        drag.landing = drag.landing(&plan, reading, Half::Lower);
        assert_eq!(drag.landing, Some(3));
        assert!(drag.among(&plan));
        drag.aim_level(&plan);
        assert_eq!(drag.level, 1, "in Word");
        let at = drag.landing.unwrap();
        assert!(place_section(
            &mut plan,
            &drag.carried,
            at,
            drag.level,
            true
        ));
        assert_eq!(
            labels(&plan),
            [
                "Welcome", "Word", "Reading", "Worship", "Song", "Hymn", "More"
            ]
        );
        // Worship holds its own two; More is still Word's.
        let held = |id: StepId| plan.within(plan.position(id).unwrap());
        assert_eq!(held(song), vec![word, worship]);
        assert_eq!(held(hymn), vec![word, worship]);
        assert_eq!(held(more), vec![word]);
    }
}
