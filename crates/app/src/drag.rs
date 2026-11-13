//! Dragging plan rows: what a drag carries and where it lands.
//!
//! A step moves alone; a section brings its steps. While a drag moves, the
//! carried rows leave the list and a gap opens where they'd land, so
//! positions here count the plan *without* them (the "rest").

use iced::Point;
use lantern_engine::{Plan, StepId};

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
}

/// How far the pointer moves before a press becomes a drag.
const SLOP: f32 = 5.0;

impl Drag {
    pub fn new(plan: &Plan, step: StepId) -> Drag {
        Drag {
            step,
            carried: plan.carried(step),
            section: plan
                .steps()
                .iter()
                .find(|s| s.id == step)
                .is_some_and(|s| s.is_section()),
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
            _ if !self.section => name,
            0 => format!("{name} · empty"),
            1 => format!("{name} · 1 step"),
            n => format!("{name} · {n} steps"),
        }
    }

    /// The pointer moved to `at`.
    pub fn to(&mut self, at: Point) {
        let origin = *self.origin.get_or_insert(at);
        self.at = at;
        self.moving |= origin.distance(at) > SLOP;
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
        if rest[at].is_section() && half == Half::Upper {
            return Some(at);
        }
        let end = rest[at + 1..]
            .iter()
            .position(|step| step.is_section())
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
    fn a_section_lands_between_sections_with_its_steps() {
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
            Some(3),
            "never splits Word"
        );
        assert_eq!(
            drag.landing(&plan, welcome, Half::Upper),
            Some(1),
            "after the loose steps"
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
}
